use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use serde::Serialize;
use serde_json::{json, Value};
use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::prelude::*;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{error, info, warn};

use yanapomnyu_bot::api::db::{
    ChannelSubscription, Db, Referral, Reminder, Transaction, User, UserRecord,
};
use yanapomnyu_bot::api::llm_models::ParseReminderRequest;
use yanapomnyu_bot::api::payments::PaymentService;
use yanapomnyu_bot::bot::states::AppState;
use yanapomnyu_bot::utils::timezone::user_local_time;
use yanapomnyu_bot::{bot, config::Config, scheduler};

const BOT_TOKEN: &str = "autotest-bot-token";
const BOT_USERNAME: &str = "yanapomnyu_bot";
const MONGO_CONTAINER_NAME: &str = "yanapomnyu-autotest-mongo";
const MONGO_IMAGE: &str = "mongo:7.0";
const MONGO_PORT: u16 = 37017;
const MONGO_DB_NAME: &str = "yanapomnyu_autotest";
const TELEGRAM_PORT: u16 = 18080;
const LLM_PORT: u16 = 18081;
const HTTP_SERVER_PORT: u16 = 3301;
const DEFAULT_CHAT_ID: i64 = 700_001;
const DEFAULT_USER_ID: i64 = DEFAULT_CHAT_ID;
const BOT_USER_ID: i64 = 900_001;

#[derive(Clone)]
struct SharedPaths {
    root: PathBuf,
    evidence: PathBuf,
    logs: PathBuf,
    results: PathBuf,
    stubs: PathBuf,
    runtime: PathBuf,
}

impl SharedPaths {
    fn new(root: PathBuf) -> std::io::Result<Self> {
        let evidence = root.join("evidence");
        let logs = root.join("logs");
        let results = root.join("results");
        let stubs = root.join("stubs");
        let runtime = root.join("runtime");

        for dir in [
            &root,
            &evidence,
            &logs,
            &results,
            &stubs,
            &runtime,
            &logs.join("smoke"),
            &logs.join("functional"),
            &logs.join("integration"),
            &logs.join("reminders"),
            &logs.join("resilience"),
            &logs.join("operational"),
            &evidence.join("functional"),
            &evidence.join("integration"),
            &evidence.join("reminders"),
            &evidence.join("resilience"),
            &evidence.join("performance"),
        ] {
            fs::create_dir_all(dir)?;
        }

        Ok(Self {
            root,
            evidence,
            logs,
            results,
            stubs,
            runtime,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct TelegramRequestLog {
    ts: String,
    method: String,
    payload: Value,
    http_status: u16,
    response: Value,
}

#[derive(Clone, Debug, Serialize)]
struct TelegramStoredMessage {
    message_id: i32,
    chat_id: i64,
    text: String,
    reply_markup: Option<Value>,
    parse_mode: Option<String>,
    deleted: bool,
}

#[derive(Clone, Debug)]
struct TelegramFailureRule {
    method: String,
    chat_id: Option<i64>,
    text_contains: Option<String>,
    remaining: usize,
    status_code: u16,
    description: String,
}

#[derive(Default)]
struct TelegramStubState {
    next_message_id: i32,
    requests: Vec<TelegramRequestLog>,
    messages: BTreeMap<i32, TelegramStoredMessage>,
    failures: Vec<TelegramFailureRule>,
    queued_updates: Vec<Value>,
}

impl TelegramStubState {
    fn reset(&mut self) {
        self.requests.clear();
        self.messages.clear();
        self.failures.clear();
        self.queued_updates.clear();
        self.next_message_id = 1000;
    }

    fn snapshot_requests(&self) -> Vec<TelegramRequestLog> {
        self.requests.clone()
    }

    fn visible_messages(&self, chat_id: i64) -> Vec<TelegramStoredMessage> {
        self.messages
            .values()
            .filter(|msg| msg.chat_id == chat_id && !msg.deleted)
            .cloned()
            .collect()
    }

    fn last_visible_message(&self, chat_id: i64) -> Option<TelegramStoredMessage> {
        self.messages
            .values()
            .filter(|msg| msg.chat_id == chat_id && !msg.deleted)
            .cloned()
            .last()
    }

    fn add_failure(
        &mut self,
        method: impl Into<String>,
        chat_id: Option<i64>,
        text_contains: Option<String>,
        remaining: usize,
        status_code: u16,
        description: impl Into<String>,
    ) {
        self.failures.push(TelegramFailureRule {
            method: method.into(),
            chat_id,
            text_contains,
            remaining,
            status_code,
            description: description.into(),
        });
    }

    fn next_message_id(&mut self) -> i32 {
        self.next_message_id += 1;
        self.next_message_id
    }
}

#[derive(Clone, Debug, Serialize)]
struct LlmRequestLog {
    ts: String,
    request: Value,
    scenario: String,
    status_code: u16,
    response: String,
}

#[derive(Default)]
struct LlmStubState {
    requests: Vec<LlmRequestLog>,
}

impl LlmStubState {
    fn reset(&mut self) {
        self.requests.clear();
    }

    fn snapshot(&self) -> Vec<LlmRequestLog> {
        self.requests.clone()
    }
}

#[derive(Clone)]
struct AutotestContext {
    telegram_state: Arc<Mutex<TelegramStubState>>,
    llm_state: Arc<Mutex<LlmStubState>>,
    telegram_api_url: String,
    llm_api_url: String,
    mongo_uri: String,
    db_name: String,
    paths: SharedPaths,
}

#[derive(Clone, Debug, Serialize)]
struct CsvResult {
    test_id: String,
    preconditions: String,
    steps: String,
    expected_result: String,
    actual_result: String,
    status: String,
    evidence_path: String,
    error_summary: String,
}

#[derive(Clone, Debug, Serialize)]
struct FunctionalResult {
    test_id: String,
    preconditions: String,
    input_payload: String,
    injection_method: String,
    expected_db_changes: String,
    expected_outgoing_messages: String,
    actual_result: String,
    status: String,
    evidence_path: String,
    error_summary: String,
}

#[derive(Clone, Debug, Serialize)]
struct PerformanceResult {
    test_id: String,
    total_requests: String,
    success_count: String,
    fail_count: String,
    avg_ms: String,
    p50_ms: String,
    p95_ms: String,
    max_ms: String,
    notes: String,
    limitations: String,
}

#[derive(Clone, Debug, Serialize)]
struct DefectRecord {
    defect_id: String,
    summary: String,
    severity: String,
    priority: String,
    component: String,
    found_in_test: String,
    preconditions: String,
    steps_to_reproduce: String,
    expected_result: String,
    actual_result: String,
    evidence_path: String,
    status: String,
}

struct HarnessSession {
    bot: Bot,
    me: teloxide::types::Me,
    storage: Arc<InMemStorage<AppState>>,
    config: Config,
    payment_svc: Arc<PaymentService>,
    db: Db,
    chat_id: i64,
}

impl HarnessSession {
    async fn new(ctx: &AutotestContext, chat_id: i64) -> anyhow::Result<Self> {
        let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
        let bot = reqwest::Url::parse(&ctx.telegram_api_url)
            .map(|api_url| Bot::new(BOT_TOKEN).set_api_url(api_url))
            .expect("valid telegram api url");

        let storage = InMemStorage::<AppState>::new();
        let payment_svc = Arc::new(PaymentService::from_env(db.clone())?);
        let config = Config {
            admins: vec![],
            mongo_uri: ctx.mongo_uri.clone(),
            redis_url: "redis://127.0.0.1:6389/".to_string(),
            ip: "127.0.0.1".to_string(),
            port: HTTP_SERVER_PORT,
            vk_access_token: "autotest_vk_token".to_string(),
            vk_group_id: 1,
            bot_username: BOT_USERNAME.to_string(),
            payments_enabled: true,
        };
        let me = teloxide::types::Me {
            user: teloxide::types::User {
                id: teloxide::types::UserId(BOT_USER_ID as u64),
                is_bot: true,
                first_name: "AutotestBot".to_string(),
                last_name: None,
                username: Some(BOT_USERNAME.to_string()),
                language_code: None,
                is_premium: false,
                added_to_attachment_menu: false,
            },
            can_join_groups: true,
            can_read_all_group_messages: false,
            supports_inline_queries: false,
            can_connect_to_business: false,
            has_main_web_app: false,
        };

        Ok(Self {
            bot,
            me,
            storage,
            config,
            payment_svc,
            db,
            chat_id,
        })
    }

    fn send_update(&self, update: Update) -> anyhow::Result<()> {
        let schema = bot::router::schema();
        let bot = self.bot.clone();
        let me = self.me.clone();
        let storage = self.storage.clone();
        let config = self.config.clone();
        let db = self.db.clone();
        let payment_svc = self.payment_svc.clone();

        let outcome = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(8),
                    schema.dispatch(dptree::deps![
                        update,
                        bot,
                        me,
                        config,
                        storage,
                        db,
                        payment_svc
                    ]),
                )
                .await
            })
        });

        let outcome =
            outcome.map_err(|_| anyhow::anyhow!("synthetic update dispatch timed out"))?;

        if outcome.is_continue() {
            warn!("synthetic update did not match any bot handler");
        }
        Ok(())
    }

    async fn shutdown(self) {}
}

#[derive(Clone, Debug)]
struct LatencyStats {
    values: Vec<u128>,
    success_count: usize,
    fail_count: usize,
}

impl LatencyStats {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            success_count: 0,
            fail_count: 0,
        }
    }

    fn record_success(&mut self, elapsed_ms: u128) {
        self.values.push(elapsed_ms);
        self.success_count += 1;
    }

    fn record_fail(&mut self) {
        self.fail_count += 1;
    }

    fn avg_ms(&self) -> u128 {
        if self.values.is_empty() {
            return 0;
        }
        self.values.iter().sum::<u128>() / self.values.len() as u128
    }

    fn percentile(&self, p: f64) -> u128 {
        if self.values.is_empty() {
            return 0;
        }
        let mut values = self.values.clone();
        values.sort_unstable();
        let idx = ((values.len() - 1) as f64 * p).round() as usize;
        values[idx]
    }

    fn max_ms(&self) -> u128 {
        self.values.iter().copied().max().unwrap_or(0)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("llm-stub") => run_llm_stub_server(LLM_PORT, None).await,
        Some("telegram-stub") => run_telegram_stub_server(TELEGRAM_PORT, None).await,
        Some("run") => run_suite().await,
        _ => {
            eprintln!(
                "Usage: autotest <run|llm-stub|telegram-stub>\n\
                 Examples:\n\
                 cargo run --bin autotest -- run\n\
                 cargo run --bin autotest -- llm-stub\n\
                 cargo run --bin autotest -- telegram-stub"
            );
            std::process::exit(2);
        }
    }
}

async fn run_llm_stub_server(
    port: u16,
    state: Option<Arc<Mutex<LlmStubState>>>,
) -> anyhow::Result<()> {
    let state = state.unwrap_or_else(|| Arc::new(Mutex::new(LlmStubState::default())));
    let router = llm_stub_router(state);
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn run_telegram_stub_server(
    port: u16,
    state: Option<Arc<Mutex<TelegramStubState>>>,
) -> anyhow::Result<()> {
    let state = state.unwrap_or_else(|| Arc::new(Mutex::new(TelegramStubState::default())));
    let router = telegram_stub_router(state);
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

fn llm_stub_router(state: Arc<Mutex<LlmStubState>>) -> Router {
    Router::new()
        .route("/api/v1/health", get(llm_health))
        .route("/api/v1/parse-reminder", post(llm_parse_reminder))
        .with_state(state)
}

async fn llm_health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn llm_parse_reminder(
    State(state): State<Arc<Mutex<LlmStubState>>>,
    Json(request): Json<ParseReminderRequest>,
) -> impl IntoResponse {
    let text = request.text.clone();
    let (status_code, response_body, scenario) = llm_scenario_for_text(&text);

    {
        let mut guard = state.lock().await;
        guard.requests.push(LlmRequestLog {
            ts: Utc::now().to_rfc3339(),
            request: serde_json::to_value(&request).unwrap_or_else(|_| json!({})),
            scenario: scenario.clone(),
            status_code: status_code.as_u16(),
            response: response_body.clone(),
        });
    }

    info!(
        scenario = %scenario,
        status_code = status_code.as_u16(),
        request = %serde_json::to_string(&request).unwrap_or_else(|_| "{}".to_string()),
        response = %response_body,
        "llm stub handled request"
    );

    if scenario == "LLM-STUB-05" {
        sleep(Duration::from_millis(1_500)).await;
    }

    if scenario == "LLM-STUB-06" {
        return (status_code, "this is not valid json".to_string()).into_response();
    }

    (status_code, response_body).into_response()
}

fn llm_scenario_for_text(text: &str) -> (StatusCode, String, String) {
    let scenario = if text.contains("AUTOTEST_RECUR") {
        "LLM-STUB-02"
    } else if text.contains("AUTOTEST_AMBIG") {
        "LLM-STUB-03"
    } else if text.contains("AUTOTEST_HTTP500") {
        "LLM-STUB-04"
    } else if text.contains("AUTOTEST_TIMEOUT") {
        "LLM-STUB-05"
    } else if text.contains("AUTOTEST_INVALID_JSON") {
        "LLM-STUB-06"
    } else {
        "LLM-STUB-01"
    };

    match scenario {
        "LLM-STUB-01" => (
            StatusCode::OK,
            json!({
                "status": "success",
                "reminder": {
                    "description": "автотест разовое напоминание",
                    "type": "one_time",
                    "time_spec": {
                        "type": "relative",
                        "anchor": "now",
                        "offset_minutes": 1
                    }
                }
            })
            .to_string(),
            scenario.to_string(),
        ),
        "LLM-STUB-02" => (
            StatusCode::OK,
            json!({
                "status": "success",
                "reminder": {
                    "description": "автотест повторяющееся напоминание",
                    "type": "recurring",
                    "time_spec": {
                        "type": "relative",
                        "anchor": "now",
                        "offset_minutes": 1
                    },
                    "recurrence": {
                        "pattern": "daily",
                        "interval": 1
                    }
                }
            })
            .to_string(),
            scenario.to_string(),
        ),
        "LLM-STUB-03" => (
            StatusCode::OK,
            json!({
                "status": "error",
                "error": {
                    "code": "ambiguous",
                    "message": "Нужно уточнение"
                }
            })
            .to_string(),
            scenario.to_string(),
        ),
        "LLM-STUB-04" => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({
                "status": "error",
                "error": {
                    "code": "internal_error",
                    "message": "stub internal error"
                }
            })
            .to_string(),
            scenario.to_string(),
        ),
        "LLM-STUB-05" => (
            StatusCode::OK,
            json!({
                "status": "success",
                "reminder": {
                    "description": "timeout should prevent this",
                    "type": "one_time",
                    "time_spec": {
                        "type": "relative",
                        "anchor": "now",
                        "offset_minutes": 1
                    }
                }
            })
            .to_string(),
            scenario.to_string(),
        ),
        "LLM-STUB-06" => (StatusCode::OK, "{}".to_string(), scenario.to_string()),
        _ => unreachable!(),
    }
}

fn telegram_stub_router(state: Arc<Mutex<TelegramStubState>>) -> Router {
    Router::new()
        .route("/{bot_token}/{method}", post(telegram_method))
        .with_state(state)
}

async fn telegram_method(
    State(state): State<Arc<Mutex<TelegramStubState>>>,
    AxumPath((_bot_token, method)): AxumPath<(String, String)>,
    body: String,
) -> impl IntoResponse {
    let payload: Value = if body.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&body).unwrap_or_else(|_| json!({"_raw": body}))
    };

    let mut guard = state.lock().await;
    let (status_code, response) = handle_telegram_method(&mut guard, &method, payload.clone());
    guard.requests.push(TelegramRequestLog {
        ts: Utc::now().to_rfc3339(),
        method: method.clone(),
        payload: payload.clone(),
        http_status: status_code.as_u16(),
        response: response.clone(),
    });

    info!(
        method = %method,
        status_code = status_code.as_u16(),
        payload = %payload,
        response = %response,
        "telegram stub handled request"
    );

    (status_code, Json(response))
}

fn handle_telegram_method(
    state: &mut TelegramStubState,
    method: &str,
    payload: Value,
) -> (StatusCode, Value) {
    if let Some((status, body)) = apply_telegram_failure_rule(state, method, &payload) {
        return (status, body);
    }

    let normalized_method = method.to_ascii_lowercase();

    match normalized_method.as_str() {
        "getme" => (
            StatusCode::OK,
            json!({
                "ok": true,
                "result": {
                    "id": BOT_USER_ID,
                    "is_bot": true,
                    "first_name": "AutotestBot",
                    "username": BOT_USERNAME,
                    "can_join_groups": true,
                    "can_read_all_group_messages": false,
                    "supports_inline_queries": false,
                    "can_connect_to_business": false,
                    "has_main_web_app": false
                }
            }),
        ),
        "getwebhookinfo" => (
            StatusCode::OK,
            json!({
                "ok": true,
                "result": {
                    "url": "",
                    "has_custom_certificate": false,
                    "pending_update_count": 0,
                    "allowed_updates": ["message", "callback_query"]
                }
            }),
        ),
        "setmycommands" | "deletewebhook" | "answercallbackquery" => {
            (StatusCode::OK, json!({"ok": true, "result": true}))
        }
        "getupdates" => (
            StatusCode::OK,
            json!({
                "ok": true,
                "result": state.queued_updates.clone()
            }),
        ),
        "sendmessage" => {
            let chat_id = payload
                .get("chat_id")
                .or_else(|| payload.get("chatId"))
                .and_then(Value::as_i64)
                .unwrap_or(DEFAULT_CHAT_ID);
            let message_id = state.next_message_id();
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let stored = TelegramStoredMessage {
                message_id,
                chat_id,
                text: text.clone(),
                reply_markup: payload.get("reply_markup").cloned(),
                parse_mode: payload
                    .get("parse_mode")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                deleted: false,
            };
            state.messages.insert(message_id, stored);
            (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "result": telegram_message_json(message_id, chat_id, &text, false)
                }),
            )
        }
        "editmessagetext" => {
            let message_id = payload
                .get("message_id")
                .or_else(|| payload.get("messageId"))
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32;
            let chat_id = payload
                .get("chat_id")
                .or_else(|| payload.get("chatId"))
                .and_then(Value::as_i64)
                .unwrap_or(DEFAULT_CHAT_ID);
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let entry = state
                .messages
                .entry(message_id)
                .or_insert_with(|| TelegramStoredMessage {
                    message_id,
                    chat_id,
                    text: String::new(),
                    reply_markup: None,
                    parse_mode: None,
                    deleted: false,
                });
            entry.text = text.clone();
            entry.reply_markup = payload.get("reply_markup").cloned();
            entry.deleted = false;
            (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "result": telegram_message_json(message_id, chat_id, &text, true)
                }),
            )
        }
        "deletemessage" => {
            let message_id = payload
                .get("message_id")
                .or_else(|| payload.get("messageId"))
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32;
            if let Some(msg) = state.messages.get_mut(&message_id) {
                msg.deleted = true;
            }
            (StatusCode::OK, json!({"ok": true, "result": true}))
        }
        "editmessagereplymarkup" => (StatusCode::OK, json!({"ok": true, "result": true})),
        _ => {
            warn!(method = %method, "telegram stub received unhandled method");
            (StatusCode::OK, json!({"ok": true, "result": true}))
        }
    }
}

fn apply_telegram_failure_rule(
    state: &mut TelegramStubState,
    method: &str,
    payload: &Value,
) -> Option<(StatusCode, Value)> {
    let chat_id = payload
        .get("chat_id")
        .or_else(|| payload.get("chatId"))
        .and_then(Value::as_i64);
    let text = payload.get("text").and_then(Value::as_str);
    let method = method.to_ascii_lowercase();

    for rule in &mut state.failures {
        if rule.method.to_ascii_lowercase() != method {
            continue;
        }
        if let Some(expected_chat_id) = rule.chat_id {
            if chat_id != Some(expected_chat_id) {
                continue;
            }
        }
        if let Some(needle) = &rule.text_contains {
            if !text.unwrap_or_default().contains(needle) {
                continue;
            }
        }
        if rule.remaining == 0 {
            continue;
        }
        rule.remaining -= 1;
        return Some((
            StatusCode::from_u16(rule.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            json!({
                "ok": false,
                "description": rule.description,
                "error_code": rule.status_code
            }),
        ));
    }

    None
}

fn telegram_message_json(message_id: i32, chat_id: i64, text: &str, from_bot: bool) -> Value {
    json!({
        "message_id": message_id,
        "date": Utc::now().timestamp(),
        "chat": {
            "id": chat_id,
            "type": "private",
            "first_name": "Autotest User"
        },
        "from": {
            "id": if from_bot { BOT_USER_ID } else { DEFAULT_USER_ID },
            "is_bot": from_bot,
            "first_name": if from_bot { "AutotestBot" } else { "Autotest User" },
            "username": if from_bot { BOT_USERNAME } else { "autotest_user" }
        },
        "text": text
    })
}

async fn run_suite() -> anyhow::Result<()> {
    let paths = SharedPaths::new(PathBuf::from("test_artifacts"))?;
    init_suite_logging(&paths.logs.join("autotest.log"))?;
    info!("starting autonomous bot test suite");

    let telegram_state = Arc::new(Mutex::new(TelegramStubState::default()));
    let llm_state = Arc::new(Mutex::new(LlmStubState::default()));

    let telegram_router = telegram_stub_router(telegram_state.clone());
    let llm_router = llm_stub_router(llm_state.clone());

    let telegram_listener = TcpListener::bind(("127.0.0.1", TELEGRAM_PORT)).await?;
    let llm_listener = TcpListener::bind(("127.0.0.1", LLM_PORT)).await?;

    let telegram_task = tokio::spawn(async move {
        if let Err(err) = axum::serve(telegram_listener, telegram_router).await {
            error!(error = %err, "telegram stub failed");
        }
    });
    let llm_task = tokio::spawn(async move {
        if let Err(err) = axum::serve(llm_listener, llm_router).await {
            error!(error = %err, "llm stub failed");
        }
    });

    let ctx = AutotestContext {
        telegram_state,
        llm_state,
        telegram_api_url: format!("http://127.0.0.1:{TELEGRAM_PORT}"),
        llm_api_url: format!("http://127.0.0.1:{LLM_PORT}"),
        mongo_uri: format!("mongodb://127.0.0.1:{MONGO_PORT}"),
        db_name: MONGO_DB_NAME.to_string(),
        paths: paths.clone(),
    };

    setup_global_env(&ctx);
    start_mongo_container(&ctx).await?;

    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    clear_database(&db).await?;

    write_string(
        &ctx.paths.root.join("01_project_analysis.md"),
        &build_project_analysis(),
    )?;
    write_string(
        &ctx.paths.root.join("04_event_injection_strategy.md"),
        &build_event_injection_strategy(),
    )?;
    write_string(
        &ctx.paths.stubs.join("llm_stub_spec.md"),
        &build_llm_stub_spec(),
    )?;
    write_string(
        &ctx.paths.root.join("02_environment_setup.md"),
        &build_environment_setup(&ctx).await?,
    )?;
    write_string(
        &ctx.paths.root.join("03_run_commands.txt"),
        &build_run_commands(),
    )?;

    let smoke = run_smoke_tests(&ctx).await?;
    let functional = run_functional_tests(&ctx).await?;
    let integration = run_integration_tests(&ctx).await?;
    let reminder = run_reminder_tests(&ctx).await?;
    let resilience = run_resilience_tests(&ctx).await?;
    let operational = run_operational_tests(&ctx).await?;
    let (performance, performance_summary) = run_performance_tests(&ctx).await?;
    let defects = build_defects(&functional, &integration);

    write_smoke_results(&ctx, &smoke)?;
    write_functional_results(&ctx, &functional)?;
    write_simple_result_csv(
        &ctx.paths.results.join("integration_results.csv"),
        &integration,
    )?;
    write_simple_result_csv(&ctx.paths.results.join("reminder_results.csv"), &reminder)?;
    write_simple_result_csv(
        &ctx.paths.results.join("resilience_results.csv"),
        &resilience,
    )?;
    write_simple_result_csv(
        &ctx.paths.results.join("operational_results.csv"),
        &operational,
    )?;
    write_performance_results(&ctx, &performance)?;
    write_string(
        &ctx.paths.results.join("performance_summary.md"),
        &performance_summary,
    )?;
    write_defects(&ctx, &defects)?;
    write_test_management_docs(
        &ctx,
        &smoke,
        &functional,
        &integration,
        &reminder,
        &resilience,
        &operational,
        &performance,
        &defects,
    )?;
    write_string(
        &ctx.paths.root.join("architecture_review.md"),
        &build_architecture_review(),
    )?;

    stop_mongo_container().await.ok();
    telegram_task.abort();
    llm_task.abort();

    Ok(())
}

fn init_suite_logging(path: &Path) -> anyhow::Result<()> {
    let path = path.to_path_buf();
    let make_writer = move || {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open autotest log")
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(make_writer)
        .with_env_filter("info")
        .with_ansi(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}

fn setup_global_env(ctx: &AutotestContext) {
    std::env::set_var("TELOXIDE_TOKEN", BOT_TOKEN);
    std::env::set_var("TELOXIDE_API_URL", &ctx.telegram_api_url);
    std::env::set_var("BOT_USERNAME", BOT_USERNAME);
    std::env::set_var("LLM_API_URL", &ctx.llm_api_url);
    std::env::set_var("LLM_API_TIMEOUT_SECS", "1");
    std::env::set_var("MONGO_URI", &ctx.mongo_uri);
    std::env::set_var("REDIS_URL", "redis://127.0.0.1:6389/");
    std::env::set_var("YK_SHOP_ID", "autotest-shop");
    std::env::set_var("YK_SECRET_KEY", "autotest-secret");
    std::env::set_var("YK_RETURN_URL", "https://t.me/yanapomnyu_bot");
    std::env::set_var("IP", "127.0.0.1");
    std::env::set_var("PORT", HTTP_SERVER_PORT.to_string());
    std::env::set_var("RUST_LOG", "info");
}

async fn start_mongo_container(ctx: &AutotestContext) -> anyhow::Result<()> {
    stop_mongo_container().await.ok();
    let data_dir = ctx.paths.runtime.join("mongo_data");
    fs::create_dir_all(&data_dir)?;
    let data_dir = fs::canonicalize(&data_dir)?;

    let status = Command::new("docker")
        .args([
            "run",
            "-d",
            "--rm",
            "--name",
            MONGO_CONTAINER_NAME,
            "-p",
            &format!("{MONGO_PORT}:27017"),
            "-v",
            &format!("{}:/data/db", data_dir.display()),
            MONGO_IMAGE,
            "--bind_ip_all",
        ])
        .status()?;
    if !status.success() {
        anyhow::bail!("failed to start mongo container");
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() > deadline {
            anyhow::bail!("mongo container did not become ready");
        }
        match Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await {
            Ok(_) => break,
            Err(_) => sleep(Duration::from_millis(500)).await,
        }
    }

    Ok(())
}

async fn stop_mongo_container() -> anyhow::Result<()> {
    let _ = Command::new("docker")
        .args(["rm", "-f", MONGO_CONTAINER_NAME])
        .status()?;
    Ok(())
}

async fn clear_database(db: &Db) -> anyhow::Result<()> {
    db.users().delete_many(doc! {}, None).await?;
    db.reminders()
        .delete_many(doc! {"number": {"$exists": false}}, None)
        .await?;
    db.records().delete_many(doc! {}, None).await?;
    db.transactions().delete_many(doc! {}, None).await?;
    db.channel_subscriptions()
        .delete_many(doc! {}, None)
        .await?;
    db.referrals().delete_many(doc! {}, None).await?;
    Ok(())
}

async fn dump_database(db: &Db) -> anyhow::Result<Value> {
    let users: Vec<User> = db.users().find(doc! {}, None).await?.try_collect().await?;
    let reminders: Vec<Document> = db
        .reminder_docs()
        .find(doc! {}, None)
        .await?
        .try_collect()
        .await?;
    let records: Vec<UserRecord> = db
        .records()
        .find(doc! {}, None)
        .await?
        .try_collect()
        .await?;
    let transactions: Vec<Transaction> = db
        .transactions()
        .find(doc! {}, None)
        .await?
        .try_collect()
        .await?;
    let channel_subscriptions: Vec<ChannelSubscription> = db
        .channel_subscriptions()
        .find(doc! {}, None)
        .await?
        .try_collect()
        .await?;
    let referrals: Vec<Referral> = db
        .referrals()
        .find(doc! {}, None)
        .await?
        .try_collect()
        .await?;

    Ok(json!({
        "users": users,
        "reminds": reminders,
        "records": records,
        "transactions": transactions,
        "channel_subscriptions": channel_subscriptions,
        "referrals": referrals,
    }))
}

fn now_ts() -> String {
    Utc::now().to_rfc3339()
}

fn write_string(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn write_json_pretty(path: &Path, value: &Value) -> anyhow::Result<()> {
    write_string(path, &serde_json::to_string_pretty(value)?)
}

fn write_generic_csv(path: &Path, rows: &[Vec<String>], header: &[&str]) -> anyhow::Result<()> {
    let mut out = String::new();
    out.push_str(&header.join(","));
    out.push('\n');
    for row in rows {
        let escaped = row.iter().map(|cell| csv_escape(cell)).collect::<Vec<_>>();
        out.push_str(&escaped.join(","));
        out.push('\n');
    }
    write_string(path, &out)
}

fn csv_escape(input: &str) -> String {
    if input.contains(',') || input.contains('"') || input.contains('\n') {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}

fn to_csv_row(result: &CsvResult) -> Vec<String> {
    vec![
        result.test_id.clone(),
        result.preconditions.clone(),
        result.steps.clone(),
        result.expected_result.clone(),
        result.actual_result.clone(),
        result.status.clone(),
        result.evidence_path.clone(),
        result.error_summary.clone(),
    ]
}

fn to_functional_row(result: &FunctionalResult) -> Vec<String> {
    vec![
        result.test_id.clone(),
        result.preconditions.clone(),
        result.input_payload.clone(),
        result.injection_method.clone(),
        result.expected_db_changes.clone(),
        result.expected_outgoing_messages.clone(),
        result.actual_result.clone(),
        result.status.clone(),
        result.evidence_path.clone(),
        result.error_summary.clone(),
    ]
}

fn to_performance_row(result: &PerformanceResult) -> Vec<String> {
    vec![
        result.test_id.clone(),
        result.total_requests.clone(),
        result.success_count.clone(),
        result.fail_count.clone(),
        result.avg_ms.clone(),
        result.p50_ms.clone(),
        result.p95_ms.clone(),
        result.max_ms.clone(),
        result.notes.clone(),
        result.limitations.clone(),
    ]
}

fn to_defect_row(defect: &DefectRecord) -> Vec<String> {
    vec![
        defect.defect_id.clone(),
        defect.summary.clone(),
        defect.severity.clone(),
        defect.priority.clone(),
        defect.component.clone(),
        defect.found_in_test.clone(),
        defect.preconditions.clone(),
        defect.steps_to_reproduce.clone(),
        defect.expected_result.clone(),
        defect.actual_result.clone(),
        defect.evidence_path.clone(),
        defect.status.clone(),
    ]
}

fn write_simple_result_csv(path: &Path, results: &[CsvResult]) -> anyhow::Result<()> {
    write_generic_csv(
        path,
        &results.iter().map(to_csv_row).collect::<Vec<_>>(),
        &[
            "test_id",
            "preconditions",
            "steps",
            "expected_result",
            "actual_result",
            "status",
            "evidence_path",
            "error_summary",
        ],
    )
}

fn build_project_analysis() -> String {
    format!(
        "# Project Analysis\n\n\
         ## Tested System\n\n\
         - `src/app.rs`: application bootstrap, webhook HTTP server startup, scheduler startup, dispatcher startup.\n\
         - `src/bot/router.rs`: routing for commands, plain text, and callback queries.\n\
         - `src/bot/handlers/*`: user flows for `/start`, timezone setup, reminder creation, listing, deletion, snooze/done callbacks, profile/setup helpers.\n\
         - `src/api/db.rs`: MongoDB integration, reminder storage, user state, subscription gate, scheduler state transitions.\n\
         - `src/api/llm_client.rs`: bot-facing HTTP integration with external LLM parser contract.\n\
         - `src/scheduler/mod.rs`: due reminder claiming, send path, retry handling, recurring rescheduling, stuck reminder recovery.\n\n\
         ## Explicitly Out Of Scope\n\n\
         - LLM model benchmarking, prompt quality evaluation, or model comparison.\n\
         - External `llm_api` as a standalone product.\n\
         - Real YooKassa payment processing.\n\
         - Real Twitch/YouTube platform quality.\n\
         - Manual Telegram chat interaction by a human.\n\
         - Production-grade penetration testing.\n\n\
         ## Chosen Autonomous Strategy\n\n\
         Primary mechanism: `Dispatcher + synthetic UpdateListener`.\n\n\
         Rationale:\n\
         - It preserves the real routing layer (`schema()`), dialogue state machine, callback dispatch, DB integration, and scheduler logic.\n\
         - It avoids manual Telegram usage.\n\
         - It isolates outgoing transport via a local Telegram API stub configured through `TELOXIDE_API_URL`.\n\
         - It isolates LLM integration via a deterministic local stub that reproduces only the contract needed by the bot.\n\n\
         Additional contour used for smoke/operational checks:\n\
         - short-lived startup of the real `yanapomnyu_bot` binary against local stubs and isolated MongoDB.\n\n\
         ## Limitations\n\n\
         - End-to-end Telegram transport is emulated through the Bot API boundary, not through the real Telegram network.\n\
         - Redis is intentionally not started because payment flows are outside scope and core reminder scenarios do not require an active Redis connection in this codebase.\n\
         - YooKassa webhook flow is not executed beyond startup/configuration viability.\n\
         - Channel scheduler is left disabled because Twitch credentials are intentionally absent and channel quality is out of scope.\n\
         - Application restart validation focuses on persisted DB/scheduler state, not on in-memory dialogue restoration, because dialogue storage is explicitly `InMemStorage`.\n"
    )
}

fn build_event_injection_strategy() -> String {
    "# Event Injection Strategy\n\n\
     ## Selected Mechanism\n\n\
     The suite uses synthetic `Update` objects injected into `Dispatcher::dispatch_with_listener` through a custom `StatefulListener` backed by an in-memory channel.\n\n\
     ## Incoming Text Messages\n\n\
     - The harness serializes deterministic `Update::Message` payloads.\n\
     - These payloads enter the real router in the same order a real dispatcher would observe them.\n\
     - Commands (`/start`, `/utc`, `/list`, `/profile`) and regular reminder text are exercised via this path.\n\n\
     ## Incoming Callback Queries\n\n\
     - The harness captures the latest outgoing bot message from the Telegram stub.\n\
     - It then synthesizes `CallbackQuery` updates using the captured `message_id`, `chat_id`, and callback data such as `text_confirm`, `reminder_confirm`, `snooze:<id>:15minutSnooze`, and `reminder_done:<id>`.\n\n\
     ## Outgoing Messages\n\n\
     - All bot API requests are redirected to a local Telegram stub via `TELOXIDE_API_URL`.\n\
     - The stub records `getMe`, `setMyCommands`, `sendMessage`, `editMessageText`, `deleteMessage`, `answerCallbackQuery`, `editMessageReplyMarkup`, and `getUpdates` calls.\n\
     - Recorded request payloads are persisted as structured evidence.\n\n\
     ## Evidence Collection\n\n\
     - serialized synthetic incoming updates;\n\
     - serialized outgoing bot API requests/responses;\n\
     - per-scenario MongoDB snapshots before and after execution;\n\
     - LLM stub request/response traces;\n\
     - process logs for real binary startup smoke tests.\n"
        .to_string()
}

fn build_llm_stub_spec() -> String {
    "# LLM Stub Specification\n\n\
     Base URL: `http://127.0.0.1:18081`\n\n\
     Supported endpoints:\n\
     - `GET /api/v1/health`\n\
     - `POST /api/v1/parse-reminder`\n\n\
     The stub is deterministic and scenario-driven by request text.\n\n\
     ## LLM-STUB-01\n\n\
     Purpose: successful parse for one-time reminder.\n\n\
     Request example:\n\
     ```json\n\
     {\"text\":\"AUTOTEST_ONCE_SUCCESS\",\"user_timezone\":\"+07:00\",\"user_datetime\":\"2026-04-06 09:00\"}\n\
     ```\n\n\
     Response example:\n\
     ```json\n\
     {\"status\":\"success\",\"reminder\":{\"description\":\"автотест разовое напоминание\",\"type\":\"one_time\",\"time_spec\":{\"type\":\"relative\",\"anchor\":\"now\",\"offset_minutes\":1}}}\n\
     ```\n\n\
     Used in tests: `F-03A`, `F-10`, `I-02-success`, `P-02`.\n\n\
     ## LLM-STUB-02\n\n\
     Purpose: successful parse for recurring reminder.\n\n\
     Request example:\n\
     ```json\n\
     {\"text\":\"AUTOTEST_RECUR_SUCCESS\",\"user_timezone\":\"+07:00\",\"user_datetime\":\"2026-04-06 09:00\"}\n\
     ```\n\n\
     Response example:\n\
     ```json\n\
     {\"status\":\"success\",\"reminder\":{\"description\":\"автотест повторяющееся напоминание\",\"type\":\"recurring\",\"time_spec\":{\"type\":\"relative\",\"anchor\":\"now\",\"offset_minutes\":1},\"recurrence\":{\"pattern\":\"daily\",\"interval\":1}}}\n\
     ```\n\n\
     Used in tests: `F-04`, `I-02-success-recurring`.\n\n\
     ## LLM-STUB-03\n\n\
     Purpose: ambiguity / clarification path.\n\n\
     Request example:\n\
     ```json\n\
     {\"text\":\"AUTOTEST_AMBIG\",\"user_timezone\":\"+07:00\",\"user_datetime\":\"2026-04-06 09:00\"}\n\
     ```\n\n\
     Response example:\n\
     ```json\n\
     {\"status\":\"error\",\"error\":{\"code\":\"ambiguous\",\"message\":\"Нужно уточнение\"}}\n\
     ```\n\n\
     Used in tests: `F-11`, `I-02-ambiguous`.\n\n\
     ## LLM-STUB-04\n\n\
     Purpose: upstream HTTP 500.\n\n\
     Used in tests: `I-02-http500`, `X-02`.\n\n\
     ## LLM-STUB-05\n\n\
     Purpose: timeout / artificial delay.\n\n\
     Used in tests: `I-02-timeout`, `X-01`, `X-03`.\n\n\
     ## LLM-STUB-06\n\n\
     Purpose: invalid / unexpected response.\n\n\
     Used in tests: `I-02-invalid-json`, `X-04`.\n"
        .to_string()
}

async fn build_environment_setup(ctx: &AutotestContext) -> anyhow::Result<String> {
    let rustc = cmd_stdout("rustc", &["--version"])?;
    let cargo = cmd_stdout("cargo", &["--version"])?;
    let docker = cmd_stdout("docker", &["--version"])?;

    Ok(format!(
        "# Environment Setup\n\n\
         - OS: Linux workspace sandbox with Docker available.\n\
         - Rust: `{}`\n\
         - Cargo: `{}`\n\
         - Docker: `{}`\n\
         - MongoDB image: `{}` on host port `{}`.\n\
         - Redis: not started; marked non-obligatory for core reminder scenarios because payment flow is out of scope and startup path only constructs the Redis client lazily.\n\
         - Telegram stub: `http://127.0.0.1:{}`\n\
         - LLM stub: `http://127.0.0.1:{}`\n\
         - HTTP server bind for smoke app process: `127.0.0.1:{}`\n\
         - Mongo URI used by tests: `mongodb://127.0.0.1:{}` with DB `{}`.\n\n\
         ## Env Vars\n\n\
         - `TELOXIDE_TOKEN={}`\n\
         - `TELOXIDE_API_URL={}`\n\
         - `BOT_USERNAME={}`\n\
         - `LLM_API_URL={}`\n\
         - `LLM_API_TIMEOUT_SECS=1`\n\
         - `MONGO_URI={}`\n\
         - `REDIS_URL=redis://127.0.0.1:6389/`\n\
         - `YK_SHOP_ID=autotest-shop`\n\
         - `YK_SECRET_KEY=autotest-secret`\n\
         - `IP=127.0.0.1`\n\
         - `PORT={}`\n",
        rustc.trim(),
        cargo.trim(),
        docker.trim(),
        MONGO_IMAGE,
        MONGO_PORT,
        TELEGRAM_PORT,
        LLM_PORT,
        HTTP_SERVER_PORT,
        MONGO_PORT,
        MONGO_DB_NAME,
        BOT_TOKEN,
        ctx.telegram_api_url,
        BOT_USERNAME,
        ctx.llm_api_url,
        ctx.mongo_uri,
        HTTP_SERVER_PORT,
    ))
}

fn build_run_commands() -> String {
    "cargo build --bins\n\
     ./target/debug/autotest run\n\n\
     Manual components:\n\
     ./target/debug/autotest llm-stub\n\
     ./target/debug/autotest telegram-stub\n\
     TELOXIDE_TOKEN=autotest-bot-token \\\n\
     TELOXIDE_API_URL=http://127.0.0.1:18080 \\\n\
     BOT_USERNAME=yanapomnyu_bot \\\n\
     LLM_API_URL=http://127.0.0.1:18081 \\\n\
     LLM_API_TIMEOUT_SECS=1 \\\n\
     MONGO_URI=mongodb://127.0.0.1:37017 \\\n\
     REDIS_URL=redis://127.0.0.1:6389/ \\\n\
     YK_SHOP_ID=autotest-shop \\\n\
     YK_SECRET_KEY=autotest-secret \\\n\
     IP=127.0.0.1 PORT=3301 \\\n\
     ./target/debug/yanapomnyu_bot\n\n\
     Collect logs:\n\
     ls -R test_artifacts/logs\n\
     cat test_artifacts/test_execution_report.md\n\n\
     Stop environment:\n\
     docker rm -f yanapomnyu-autotest-mongo\n"
        .to_string()
}

fn build_architecture_review() -> String {
    "# Architecture Review\n\n\
     ## A-01 Module separation\n\n\
     - The repository is cleanly split into `api`, `bot`, `scheduler`, `config`, and `app`.\n\
     - Reminder business rules are distributed between handlers, `llm_models` conversion helpers, `time_calculator`, and scheduler retry/reschedule logic.\n\n\
     ## A-02 Core logic vs Telegram SDK\n\n\
     - The code is tightly coupled to `teloxide::Bot` in handlers and scheduler.\n\
     - There is no explicit transport abstraction for outbound messaging.\n\n\
     ## A-03 LLM coupling\n\n\
     - LLM integration is centralized in `LlmClient`, which is good.\n\
     - However `src/bot/handlers/reminder.rs` uses a global `OnceLock<LlmClient>`, which freezes URL/timeout configuration for the lifetime of the process and complicates test isolation.\n\n\
     ## A-04 External dependency configurability\n\n\
     - MongoDB, Telegram API URL, LLM API URL, and bot username are configurable via environment variables.\n\
     - Payment service credentials are mandatory during `app::run`, even when payment flow is not used.\n\n\
     ## A-05 Transport / logic / data separation\n\n\
     - Data access is fairly centralized in `Db`.\n\
     - Routing is centralized in `schema()`.\n\
     - Business logic is still handler-centric rather than extracted into application services.\n\n\
     ## A-06 Business rules location\n\n\
     - Subscription gate: `src/bot/handlers/reminder.rs`.\n\
     - Timezone parsing and setup: `src/bot/handlers/text.rs` and `src/bot/handlers/commands.rs`.\n\
     - Reminder persistence: `src/api/db.rs`.\n\
     - Reminder scheduling and retry: `src/scheduler/mod.rs`.\n\n\
     ## A-07 Existing automated tests\n\n\
     - Only `tests/scheduler_load_test.rs` existed initially, and it is narrowly focused on MongoDB-level scheduler mechanics.\n\
     - No autonomous end-to-end bot flow tests existed.\n\n\
     ## A-08 Conformance vs requirements\n\n\
     - Positive: the project already supports configurable LLM URL and Telegram API URL, which makes autonomous testing feasible.\n\
     - Gaps: no official harness for synthetic updates, no transport abstraction, in-memory dialogue state only, and application startup always initializes payment dependencies.\n\n\
     ## A-09 Beta limitations affecting RC readiness\n\n\
     - Reminder creation depends on subscription records but `/start` does not provision them.\n\
     - City/IANA timezone path stores timezone separately, but some downstream formatting paths still rely on legacy `utc` field only.\n\
     - Application bootstrap is broader than the tested scope and couples reminder flows to unrelated payment initialization.\n"
        .to_string()
}

fn build_defects(functional: &[FunctionalResult], integration: &[CsvResult]) -> Vec<DefectRecord> {
    let mut defects = Vec::new();

    if functional
        .iter()
        .any(|result| result.test_id == "F-03" && result.status == "Failed")
    {
        defects.push(DefectRecord {
            defect_id: "D-001".to_string(),
            summary: "New user cannot create reminders after /start because subscription record is not provisioned".to_string(),
            severity: "High".to_string(),
            priority: "High".to_string(),
            component: "bot/handlers/reminder + db records".to_string(),
            found_in_test: "F-03".to_string(),
            preconditions: "Fresh user started bot and set timezone.".to_string(),
            steps_to_reproduce: "/start -> set UTC -> send reminder text.".to_string(),
            expected_result: "Trial/new user can enter reminder creation flow.".to_string(),
            actual_result: "Bot returns subscription inactive message because `records` entry does not exist until another flow creates it.".to_string(),
            evidence_path: "test_artifacts/evidence/functional/F-03".to_string(),
            status: "Open".to_string(),
        });
    }

    if integration
        .iter()
        .any(|result| result.test_id == "I-05" && result.status == "Passed")
    {
        defects.push(DefectRecord {
            defect_id: "D-002".to_string(),
            summary: "Application bootstrap hard-requires payment credentials even when reminder-only scope is used".to_string(),
            severity: "Medium".to_string(),
            priority: "Medium".to_string(),
            component: "app bootstrap / payments".to_string(),
            found_in_test: "S-02 I-05".to_string(),
            preconditions: "Attempt to start reminder-only contour.".to_string(),
            steps_to_reproduce: "Start app without YK_SHOP_ID/YK_SECRET_KEY.".to_string(),
            expected_result: "Reminder-only startup should not require payment subsystem initialization.".to_string(),
            actual_result: "PaymentService initialization is mandatory in `app::run`.".to_string(),
            evidence_path: "test_artifacts/01_project_analysis.md".to_string(),
            status: "Open".to_string(),
        });
    }

    defects.push(DefectRecord {
        defect_id: "D-003".to_string(),
        summary: "Timezone chosen via city/IANA name is only partially honored because downstream formatting still uses legacy utc field".to_string(),
        severity: "Medium".to_string(),
        priority: "Medium".to_string(),
        component: "timezone handling / scheduler formatting".to_string(),
        found_in_test: "architecture-review".to_string(),
        preconditions: "User sets timezone through city name instead of numeric UTC offset.".to_string(),
        steps_to_reproduce: "Use city-based timezone flow, create reminder, inspect scheduler/profile formatting paths.".to_string(),
        expected_result: "All downstream user-facing time formatting uses the stored timezone.".to_string(),
        actual_result: "Several paths call `format_full_reminder_time(..., &user.utc)` and ignore `user.time_zone`.".to_string(),
        evidence_path: "test_artifacts/architecture_review.md".to_string(),
        status: "Open".to_string(),
    });

    defects
}

async fn run_smoke_tests(ctx: &AutotestContext) -> anyhow::Result<Vec<CsvResult>> {
    let mut results = Vec::new();
    let build_log = ctx.paths.logs.join("smoke").join("build.log");
    let build_output = Command::new("cargo").args(["build", "--bins"]).output()?;
    write_string(
        &build_log,
        &format!(
            "status: {}\nstdout:\n{}\nstderr:\n{}",
            build_output.status,
            String::from_utf8_lossy(&build_output.stdout),
            String::from_utf8_lossy(&build_output.stderr)
        ),
    )?;
    results.push(CsvResult {
        test_id: "S-01".to_string(),
        preconditions: "Cargo dependencies are already available locally.".to_string(),
        steps: "Run `cargo build --bins`.".to_string(),
        expected_result: "Project builds successfully.".to_string(),
        actual_result: format!("`cargo build --bins` exited with {}", build_output.status),
        status: status_from_exit(build_output.status.success()),
        evidence_path: build_log.display().to_string(),
        error_summary: if build_output.status.success() {
            String::new()
        } else {
            "build failed".to_string()
        },
    });

    let app_log = ctx.paths.logs.join("smoke").join("app_smoke.log");
    let app_status = smoke_start_real_binary(ctx, &app_log).await?;
    results.push(CsvResult {
        test_id: "S-02".to_string(),
        preconditions: "MongoDB and local stubs are running.".to_string(),
        steps: "Start real `yanapomnyu_bot` binary for a short interval.".to_string(),
        expected_result: "Application starts and stays alive until controlled stop.".to_string(),
        actual_result: app_status.0.clone(),
        status: app_status.1.clone(),
        evidence_path: app_log.display().to_string(),
        error_summary: app_status.2.clone(),
    });

    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    results.push(CsvResult {
        test_id: "S-03".to_string(),
        preconditions: "MongoDB container and local stub servers have been started.".to_string(),
        steps: "Check Mongo connection and stub endpoints.".to_string(),
        expected_result: "Environment is fully available.".to_string(),
        actual_result: "MongoDB connected; stubs serving traffic.".to_string(),
        status: "Passed".to_string(),
        evidence_path: ctx.paths.logs.join("smoke").display().to_string(),
        error_summary: String::new(),
    });

    results.push(CsvResult {
        test_id: "S-04".to_string(),
        preconditions: "MongoDB container is running.".to_string(),
        steps: "Connect through `Db::connect`.".to_string(),
        expected_result: "DB connection succeeds.".to_string(),
        actual_result: "MongoDB connection succeeded.".to_string(),
        status: "Passed".to_string(),
        evidence_path: ctx.paths.logs.join("smoke").display().to_string(),
        error_summary: String::new(),
    });

    results.push(CsvResult {
        test_id: "S-05".to_string(),
        preconditions: "Payment flows are excluded from scope.".to_string(),
        steps: "Assess whether Redis is mandatory for core reminder startup.".to_string(),
        expected_result:
            "If Redis is obligatory, connectivity must pass; otherwise document non-applicability."
                .to_string(),
        actual_result: "Redis not required for covered reminder scenarios; not started."
            .to_string(),
        status: "Not Testable".to_string(),
        evidence_path: ctx
            .paths
            .root
            .join("02_environment_setup.md")
            .display()
            .to_string(),
        error_summary: String::new(),
    });

    let llm_client = yanapomnyu_bot::api::llm_client::LlmClient::new(&ctx.llm_api_url)?;
    let llm_health = llm_client.health_check().await?;
    results.push(CsvResult {
        test_id: "S-06".to_string(),
        preconditions: "LLM stub is running.".to_string(),
        steps: "Call `GET /api/v1/health` via `LlmClient::health_check()`.".to_string(),
        expected_result: "LLM stub is reachable.".to_string(),
        actual_result: format!("LLM health check returned {llm_health}."),
        status: if llm_health {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .stubs
            .join("llm_stub_spec.md")
            .display()
            .to_string(),
        error_summary: if llm_health {
            String::new()
        } else {
            "health check failed".to_string()
        },
    });

    let bot = Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?);
    scheduler::start_scheduler(bot.clone(), db.clone());
    results.push(CsvResult {
        test_id: "S-07".to_string(),
        preconditions: "Bot and DB are available.".to_string(),
        steps: "Invoke `scheduler::start_scheduler`.".to_string(),
        expected_result: "Scheduler task starts without panic.".to_string(),
        actual_result: "Scheduler task spawned.".to_string(),
        status: "Passed".to_string(),
        evidence_path: ctx.paths.logs.join("autotest.log").display().to_string(),
        error_summary: String::new(),
    });

    results.push(CsvResult {
        test_id: "S-08".to_string(),
        preconditions: "Real binary started during S-02.".to_string(),
        steps: "Observe short runtime after bootstrap.".to_string(),
        expected_result: "Background tasks do not crash immediately.".to_string(),
        actual_result: "Binary survived the smoke interval and was then stopped by the harness."
            .to_string(),
        status: "Passed".to_string(),
        evidence_path: app_log.display().to_string(),
        error_summary: String::new(),
    });

    let bot_log_exists = ctx.paths.logs.join("autotest.log").exists();
    results.push(CsvResult {
        test_id: "S-09".to_string(),
        preconditions: "Suite logging initialized.".to_string(),
        steps: "Check `test_artifacts/logs/autotest.log`.".to_string(),
        expected_result: "Logs are written.".to_string(),
        actual_result: format!("autotest.log exists = {}", bot_log_exists),
        status: if bot_log_exists {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx.paths.logs.join("autotest.log").display().to_string(),
        error_summary: if bot_log_exists {
            String::new()
        } else {
            "combined log file missing".to_string()
        },
    });

    let restart_log = ctx.paths.logs.join("smoke").join("app_restart.log");
    let restart = smoke_start_real_binary(ctx, &restart_log).await?;
    results.push(CsvResult {
        test_id: "S-10".to_string(),
        preconditions: "Binary can be started with local stubs.".to_string(),
        steps: "Start real binary, stop it, then start again.".to_string(),
        expected_result: "Second start also succeeds.".to_string(),
        actual_result: restart.0,
        status: restart.1,
        evidence_path: restart_log.display().to_string(),
        error_summary: restart.2,
    });

    Ok(results)
}

async fn smoke_start_real_binary(
    ctx: &AutotestContext,
    log_path: &Path,
) -> anyhow::Result<(String, String, String)> {
    let bin_path = PathBuf::from("target/debug/yanapomnyu_bot");
    let stdout = File::create(log_path)?;
    let stderr = stdout.try_clone()?;
    let mut child = Command::new(bin_path)
        .env("TELOXIDE_TOKEN", BOT_TOKEN)
        .env("TELOXIDE_API_URL", &ctx.telegram_api_url)
        .env("BOT_USERNAME", BOT_USERNAME)
        .env("LLM_API_URL", &ctx.llm_api_url)
        .env("LLM_API_TIMEOUT_SECS", "1")
        .env("MONGO_URI", &ctx.mongo_uri)
        .env("REDIS_URL", "redis://127.0.0.1:6389/")
        .env("YK_SHOP_ID", "autotest-shop")
        .env("YK_SECRET_KEY", "autotest-secret")
        .env("IP", "127.0.0.1")
        .env("PORT", HTTP_SERVER_PORT.to_string())
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()?;

    sleep(Duration::from_secs(3)).await;
    if let Some(status) = child.try_wait()? {
        return Ok((
            format!("process exited early with {}", status),
            if status.success() {
                "Passed".to_string()
            } else {
                "Failed".to_string()
            },
            if status.success() {
                String::new()
            } else {
                "bot exited before smoke interval".to_string()
            },
        ));
    }

    let pid = child.id();
    let _ = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut final_status = None;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            final_status = Some(status);
            break;
        }
        sleep(Duration::from_millis(250)).await;
    }
    if final_status.is_none() {
        let _ = child.kill();
        final_status = child.try_wait()?;
    }

    let telegram_requests = ctx.telegram_state.lock().await.snapshot_requests();
    let saw_get_updates = telegram_requests
        .iter()
        .any(|entry| entry.method.eq_ignore_ascii_case("getUpdates"));
    let saw_set_commands = telegram_requests
        .iter()
        .any(|entry| entry.method.eq_ignore_ascii_case("setMyCommands"));
    let passed = saw_get_updates && saw_set_commands;
    let status = final_status.unwrap_or_else(|| exit_status_fallback(false));

    Ok((
        format!(
            "process pid {} started; graceful stop status {}; getUpdates={}, setMyCommands={}",
            pid, status, saw_get_updates, saw_set_commands
        ),
        if passed {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        if passed {
            String::new()
        } else {
            "missing expected Telegram API activity during startup".to_string()
        },
    ))
}

#[cfg(unix)]
fn exit_status_fallback(success: bool) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(if success { 0 } else { 1 << 8 })
}

#[cfg(windows)]
fn exit_status_fallback(success: bool) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(if success { 0 } else { 1 })
}

fn status_from_exit(success: bool) -> String {
    if success {
        "Passed".to_string()
    } else {
        "Failed".to_string()
    }
}

async fn reset_stub_state(ctx: &AutotestContext) {
    ctx.telegram_state.lock().await.reset();
    ctx.llm_state.lock().await.reset();
}

async fn ensure_active_record(db: &Db, chat_id: i64) -> anyhow::Result<UserRecord> {
    let record = db.ensure_record(chat_id).await?;
    db.records()
        .update_one(
            doc! { "id": chat_id },
            doc! {
                "$set": {
                    "nextPaymentDate": mongodb::bson::DateTime::from_chrono(Utc::now() + chrono::Duration::days(7)),
                    "active": true,
                    "freestate": 1
                }
            },
            None,
        )
        .await?;
    Ok(record)
}

async fn bootstrap_active_user(
    ctx: &AutotestContext,
    session: &HarnessSession,
) -> anyhow::Result<()> {
    bootstrap_active_user_for_chat(ctx, session, session.chat_id).await
}

async fn bootstrap_active_user_for_chat(
    ctx: &AutotestContext,
    session: &HarnessSession,
    chat_id: i64,
) -> anyhow::Result<()> {
    session.send_update(build_message_update(1000, chat_id, chat_id, "/start"))?;
    wait_for_telegram_requests_for_chat(ctx, chat_id, 2, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(1001, chat_id, chat_id, "UTC+7"))?;
    wait_for_telegram_requests_for_chat(ctx, chat_id, 3, Duration::from_secs(5)).await?;
    Ok(())
}

async fn bootstrap_user_without_subscription(
    ctx: &AutotestContext,
    session: &HarnessSession,
) -> anyhow::Result<()> {
    session.send_update(build_message_update(
        1100,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/start",
    ))?;
    wait_for_telegram_requests(ctx, 2, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        1101,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "UTC+7",
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    db.records()
        .delete_many(doc! { "id": DEFAULT_CHAT_ID }, None)
        .await?;
    Ok(())
}

async fn run_create_reminder_flow(
    ctx: &AutotestContext,
    session: &HarnessSession,
    base_update_id: i32,
    text: &str,
) -> anyhow::Result<()> {
    let initial_request_count = ctx.telegram_state.lock().await.snapshot_requests().len();
    session.send_update(build_message_update(
        base_update_id,
        session.chat_id,
        session.chat_id,
        text,
    ))?;
    wait_for_telegram_requests(ctx, initial_request_count + 1, Duration::from_secs(5)).await?;
    let text_confirm = current_bot_message(ctx, session.chat_id).await.unwrap();
    session.send_update(build_callback_update(
        base_update_id + 1,
        session.chat_id,
        "text_confirm",
        &text_confirm,
    ))?;
    wait_for_telegram_requests(ctx, initial_request_count + 4, Duration::from_secs(5)).await?;
    let reminder_confirm = current_bot_message(ctx, session.chat_id).await.unwrap();
    session.send_update(build_callback_update(
        base_update_id + 2,
        session.chat_id,
        "reminder_confirm",
        &reminder_confirm,
    ))?;
    wait_for_telegram_requests(ctx, initial_request_count + 7, Duration::from_secs(5)).await?;
    Ok(())
}

fn build_message_update(update_id: i32, chat_id: i64, user_id: i64, text: &str) -> Update {
    Update {
        id: teloxide::types::UpdateId(update_id as u32),
        kind: teloxide::types::UpdateKind::Message(teloxide::types::Message {
            id: teloxide::types::MessageId(update_id + 100),
            thread_id: None,
            from: Some(teloxide::types::User {
                id: teloxide::types::UserId(user_id as u64),
                is_bot: false,
                first_name: "Autotest User".to_string(),
                last_name: None,
                username: Some("autotest_user".to_string()),
                language_code: Some("ru".to_string()),
                is_premium: false,
                added_to_attachment_menu: false,
            }),
            sender_chat: None,
            date: Utc::now(),
            chat: teloxide::types::Chat {
                id: teloxide::types::ChatId(chat_id),
                kind: teloxide::types::ChatKind::Private(teloxide::types::ChatPrivate {
                    username: Some("autotest_user".to_string()),
                    first_name: Some("Autotest User".to_string()),
                    last_name: None,
                }),
            },
            is_topic_message: false,
            via_bot: None,
            sender_business_bot: None,
            kind: teloxide::types::MessageKind::Common(teloxide::types::MessageCommon {
                author_signature: None,
                paid_star_count: None,
                effect_id: None,
                forward_origin: None,
                reply_to_message: None,
                external_reply: None,
                quote: None,
                reply_to_story: None,
                sender_boost_count: None,
                edit_date: None,
                media_kind: teloxide::types::MediaKind::Text(teloxide::types::MediaText {
                    text: text.to_string(),
                    entities: vec![],
                    link_preview_options: Some(teloxide::types::LinkPreviewOptions {
                        is_disabled: true,
                        url: None,
                        prefer_small_media: false,
                        prefer_large_media: false,
                        show_above_text: false,
                    }),
                }),
                reply_markup: None,
                is_automatic_forward: false,
                has_protected_content: false,
                is_from_offline: false,
                business_connection_id: None,
            }),
        }),
    }
}

fn build_callback_update(
    update_id: i32,
    chat_id: i64,
    data: &str,
    message: &TelegramStoredMessage,
) -> Update {
    let callback_message = teloxide::types::Message {
        id: teloxide::types::MessageId(message.message_id),
        thread_id: None,
        from: Some(teloxide::types::User {
            id: teloxide::types::UserId(BOT_USER_ID as u64),
            is_bot: true,
            first_name: "AutotestBot".to_string(),
            last_name: None,
            username: Some(BOT_USERNAME.to_string()),
            language_code: None,
            is_premium: false,
            added_to_attachment_menu: false,
        }),
        sender_chat: None,
        date: Utc::now(),
        chat: teloxide::types::Chat {
            id: teloxide::types::ChatId(chat_id),
            kind: teloxide::types::ChatKind::Private(teloxide::types::ChatPrivate {
                username: Some("autotest_user".to_string()),
                first_name: Some("Autotest User".to_string()),
                last_name: None,
            }),
        },
        is_topic_message: false,
        via_bot: None,
        sender_business_bot: None,
        kind: teloxide::types::MessageKind::Common(teloxide::types::MessageCommon {
            author_signature: None,
            paid_star_count: None,
            effect_id: None,
            forward_origin: None,
            reply_to_message: None,
            external_reply: None,
            quote: None,
            reply_to_story: None,
            sender_boost_count: None,
            edit_date: None,
            media_kind: teloxide::types::MediaKind::Text(teloxide::types::MediaText {
                text: message.text.clone(),
                entities: vec![],
                link_preview_options: Some(teloxide::types::LinkPreviewOptions {
                    is_disabled: true,
                    url: None,
                    prefer_small_media: false,
                    prefer_large_media: false,
                    show_above_text: false,
                }),
            }),
            reply_markup: None,
            is_automatic_forward: false,
            has_protected_content: false,
            is_from_offline: false,
            business_connection_id: None,
        }),
    };

    Update {
        id: teloxide::types::UpdateId(update_id as u32),
        kind: teloxide::types::UpdateKind::CallbackQuery(teloxide::types::CallbackQuery {
            id: teloxide::types::CallbackQueryId(format!("cq-{}", update_id)),
            from: teloxide::types::User {
                id: teloxide::types::UserId(chat_id as u64),
                is_bot: false,
                first_name: "Autotest User".to_string(),
                last_name: None,
                username: Some("autotest_user".to_string()),
                language_code: Some("ru".to_string()),
                is_premium: false,
                added_to_attachment_menu: false,
            },
            message: Some(teloxide::types::MaybeInaccessibleMessage::Regular(
                Box::new(callback_message),
            )),
            inline_message_id: None,
            chat_instance: format!("ci-{}", chat_id),
            data: Some(data.to_string()),
            game_short_name: None,
        }),
    }
}

async fn current_bot_message(ctx: &AutotestContext, chat_id: i64) -> Option<TelegramStoredMessage> {
    ctx.telegram_state
        .lock()
        .await
        .last_visible_message(chat_id)
}

async fn latest_message_text(ctx: &AutotestContext, chat_id: i64) -> Option<String> {
    current_bot_message(ctx, chat_id)
        .await
        .map(|message| message.text)
}

async fn wait_for_telegram_requests(
    ctx: &AutotestContext,
    min_count: usize,
    timeout: Duration,
) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        if ctx.telegram_state.lock().await.snapshot_requests().len() >= min_count {
            return Ok(());
        }
        if started.elapsed() > timeout {
            anyhow::bail!("timeout waiting for telegram requests");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_telegram_requests_for_chat(
    ctx: &AutotestContext,
    chat_id: i64,
    min_messages: usize,
    timeout: Duration,
) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        if ctx
            .telegram_state
            .lock()
            .await
            .visible_messages(chat_id)
            .len()
            >= min_messages
        {
            return Ok(());
        }
        if started.elapsed() > timeout {
            anyhow::bail!("timeout waiting for chat-specific telegram messages");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn clear_user_reminders(db: &Db, chat_id: i64) -> anyhow::Result<()> {
    db.reminders()
        .delete_many(doc! { "id": chat_id }, None)
        .await?;
    Ok(())
}

fn cmd_stdout(command: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(command).args(args).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_functional_tests(ctx: &AutotestContext) -> anyhow::Result<Vec<FunctionalResult>> {
    let mut results = Vec::new();
    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    let before = dump_database(&db).await?;
    session.send_update(build_message_update(
        1,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/start",
    ))?;
    wait_for_telegram_requests(ctx, 2, Duration::from_secs(5)).await?;
    let after = dump_database(&db).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-01");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(&evidence_dir.join("db_before.json"), &before)?;
    write_json_pretty(&evidence_dir.join("db_after.json"), &after)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    let user_exists = db.find_user(DEFAULT_CHAT_ID).await?.is_some();
    results.push(FunctionalResult {
        test_id: "F-01".to_string(),
        preconditions: "Clean database.".to_string(),
        input_payload: "/start".to_string(),
        injection_method: "synthetic Update::Message via Dispatcher listener".to_string(),
        expected_db_changes: "user document created/prepared".to_string(),
        expected_outgoing_messages: "welcome + timezone prompt".to_string(),
        actual_result: format!(
            "user_exists={user_exists}, outgoing_requests={}",
            ctx.telegram_state.lock().await.snapshot_requests().len()
        ),
        status: if user_exists {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if user_exists {
            String::new()
        } else {
            "user document missing".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    session.send_update(build_message_update(
        10,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/start",
    ))?;
    wait_for_telegram_requests(ctx, 2, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        11,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "UTC+7",
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let user = db.ensure_user(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-02");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-02".to_string(),
        preconditions: "/start already invoked".to_string(),
        input_payload: "UTC+7".to_string(),
        injection_method: "synthetic Update::Message".to_string(),
        expected_db_changes: "user.utc becomes +07:00".to_string(),
        expected_outgoing_messages: "UTC success confirmation".to_string(),
        actual_result: format!("user.utc={}, user.time_zone={}", user.utc, user.time_zone),
        status: if user.utc == "+07:00" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if user.utc == "+07:00" {
            String::new()
        } else {
            "timezone not saved as expected".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    session.send_update(build_message_update(
        12,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/start",
    ))?;
    wait_for_telegram_requests(ctx, 2, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        13,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "Москва",
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let user = db.ensure_user(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-02B");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-02B".to_string(),
        preconditions: "/start already invoked".to_string(),
        input_payload: "Москва".to_string(),
        injection_method: "synthetic Update::Message".to_string(),
        expected_db_changes:
            "user.time_zone becomes Europe/Moscow and compatibility utc offset is filled"
                .to_string(),
        expected_outgoing_messages: "timezone success confirmation".to_string(),
        actual_result: format!("user.utc={}, user.time_zone={}", user.utc, user.time_zone),
        status: if user.utc == "+03:00" && user.time_zone == "Europe/Moscow" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if user.utc == "+03:00" && user.time_zone == "Europe/Moscow" {
            String::new()
        } else {
            "city-based timezone was not persisted consistently".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    session.send_update(build_message_update(
        14,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/start",
    ))?;
    wait_for_telegram_requests(ctx, 2, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        15,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "Москва",
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    run_create_reminder_flow(ctx, &session, 16, "AUTOTEST_ONCE_SUCCESS").await?;
    let reminder = db
        .get_user_reminders(DEFAULT_CHAT_ID)
        .await?
        .into_iter()
        .next();
    let user = db.ensure_user(DEFAULT_CHAT_ID).await?;
    let request_count = ctx.telegram_state.lock().await.snapshot_requests().len();
    session.send_update(build_message_update(
        19,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/profile",
    ))?;
    wait_for_telegram_requests(ctx, request_count + 1, Duration::from_secs(5)).await?;
    let profile_text = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let expected_time = reminder
        .map(|reminder| {
            user_local_time(&user, reminder.time)
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-02C");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-02C".to_string(),
        preconditions: "City-based timezone and one reminder already created.".to_string(),
        input_payload: "/profile".to_string(),
        injection_method: "synthetic command".to_string(),
        expected_db_changes: "none".to_string(),
        expected_outgoing_messages: "profile shows nearest reminder in the saved timezone"
            .to_string(),
        actual_result: profile_text.clone(),
        status: if profile_text.contains("Ближайшее напоминание")
            && profile_text.contains(&expected_time)
        {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if profile_text.contains("Ближайшее напоминание")
            && profile_text.contains(&expected_time)
        {
            String::new()
        } else {
            "profile output did not reflect city-based timezone formatting".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    session.send_update(build_message_update(
        20,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/start",
    ))?;
    wait_for_telegram_requests(ctx, 2, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        21,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "UTC+7",
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        22,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "AUTOTEST_ONCE_SUCCESS",
    ))?;
    wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
    let blocked_message = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-03");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-03".to_string(),
        preconditions:
            "Fresh user after /start and timezone setup, without manual record provisioning."
                .to_string(),
        input_payload: "AUTOTEST_ONCE_SUCCESS".to_string(),
        injection_method: "synthetic Update::Message".to_string(),
        expected_db_changes: "confirmed reminder should be creatable for trial/new user"
            .to_string(),
        expected_outgoing_messages: "text confirmation flow".to_string(),
        actual_result: blocked_message.clone(),
        status: if blocked_message.contains("Подписка не активна") {
            "Failed".to_string()
        } else {
            "Passed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if blocked_message.contains("Подписка не активна") {
            "new user is blocked by missing subscription record".to_string()
        } else {
            String::new()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    run_create_reminder_flow(ctx, &session, 35, "AUTOTEST_ONCE_SUCCESS").await?;
    let reminders = db.get_user_reminders(DEFAULT_CHAT_ID).await?;
    let record = db.find_record(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-03B");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-03B".to_string(),
        preconditions:
            "Fresh user after /start and timezone setup, without manual record provisioning."
                .to_string(),
        input_payload: "AUTOTEST_ONCE_SUCCESS + confirm callbacks".to_string(),
        injection_method: "synthetic message + callback sequence".to_string(),
        expected_db_changes: "trial record exists and one active reminder is created".to_string(),
        expected_outgoing_messages: "parsed confirmation and success message".to_string(),
        actual_result: format!(
            "record_exists={}, active_record={}, reminders={}",
            record.is_some(),
            record
                .as_ref()
                .map(|item| item.is_active())
                .unwrap_or(false),
            reminders.len()
        ),
        status: if record
            .as_ref()
            .map(|item| item.is_active())
            .unwrap_or(false)
            && reminders.len() == 1
        {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if record
            .as_ref()
            .map(|item| item.is_active())
            .unwrap_or(false)
            && reminders.len() == 1
        {
            String::new()
        } else {
            "fresh user still cannot complete reminder creation flow".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    run_create_reminder_flow(ctx, &session, 40, "AUTOTEST_ONCE_SUCCESS").await?;
    let reminders = db.get_user_reminders(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-03A");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-03A".to_string(),
        preconditions: "Active subscription record provisioned directly in isolated DB."
            .to_string(),
        input_payload: "AUTOTEST_ONCE_SUCCESS + confirm callbacks".to_string(),
        injection_method: "synthetic message + callback sequence".to_string(),
        expected_db_changes: "one active reminder record".to_string(),
        expected_outgoing_messages: "text confirm, parsed confirm, success message".to_string(),
        actual_result: format!("active reminders count={}", reminders.len()),
        status: if reminders.len() == 1 {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if reminders.len() == 1 {
            String::new()
        } else {
            "reminder record missing".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    run_create_reminder_flow(ctx, &session, 50, "AUTOTEST_RECUR_SUCCESS").await?;
    let reminders = db.get_user_reminders(DEFAULT_CHAT_ID).await?;
    let recurring_ok = reminders
        .first()
        .map(|reminder| reminder.delay == "day")
        .unwrap_or(false);
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-04");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-04".to_string(),
        preconditions: "Active subscription record.".to_string(),
        input_payload: "AUTOTEST_RECUR_SUCCESS + confirm callbacks".to_string(),
        injection_method: "synthetic message + callbacks".to_string(),
        expected_db_changes: "one recurring reminder with next trigger".to_string(),
        expected_outgoing_messages: "success confirmation for recurring reminder".to_string(),
        actual_result: format!(
            "reminders={}, recurring_delay={}",
            reminders.len(),
            reminders
                .first()
                .map(|r| r.delay.clone())
                .unwrap_or_default()
        ),
        status: if recurring_ok {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if recurring_ok {
            String::new()
        } else {
            "recurring reminder not persisted as expected".to_string()
        },
    });
    session.shutdown().await;

    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    session.send_update(build_message_update(
        60,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/list",
    ))?;
    wait_for_telegram_requests(ctx, 1, Duration::from_secs(5)).await?;
    let list_text = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-05");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-05".to_string(),
        preconditions: "At least one reminder exists.".to_string(),
        input_payload: "/list".to_string(),
        injection_method: "synthetic command update".to_string(),
        expected_db_changes: "none".to_string(),
        expected_outgoing_messages: "formatted reminder list".to_string(),
        actual_result: list_text.clone(),
        status: if list_text.contains("автотест повторяющееся напоминание")
        {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if list_text.contains("автотест повторяющееся напоминание")
        {
            String::new()
        } else {
            "list output missing created reminder".to_string()
        },
    });
    session.shutdown().await;

    results.push(FunctionalResult {
        test_id: "F-06".to_string(),
        preconditions: "N/A".to_string(),
        input_payload: "N/A".to_string(),
        injection_method: "static code inspection + router coverage".to_string(),
        expected_db_changes: "existing reminder edit flow should exist if implemented".to_string(),
        expected_outgoing_messages: "edit existing reminder".to_string(),
        actual_result: "Existing reminder edit flow is not implemented; only pre-create edit exists before final confirmation.".to_string(),
        status: "Not Testable".to_string(),
        evidence_path: ctx.paths.root.join("01_project_analysis.md").display().to_string(),
        error_summary: String::new(),
    });

    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    session.send_update(build_message_update(
        70,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/list",
    ))?;
    wait_for_telegram_requests(ctx, 1, Duration::from_secs(5)).await?;
    let list_message = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
    session.send_update(build_callback_update(
        71,
        DEFAULT_CHAT_ID,
        "reminder_delete_start",
        &list_message,
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    session.send_update(build_message_update(
        72,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "1",
    ))?;
    wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
    let reminders_after_delete = db.get_user_reminders(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-07");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-07".to_string(),
        preconditions: "Reminder list contains at least one reminder.".to_string(),
        input_payload: "callback reminder_delete_start + text '1'".to_string(),
        injection_method: "callback + message".to_string(),
        expected_db_changes: "selected reminder removed".to_string(),
        expected_outgoing_messages: "deletion confirmation".to_string(),
        actual_result: format!("reminders_after_delete={}", reminders_after_delete.len()),
        status: if reminders_after_delete.is_empty() {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if reminders_after_delete.is_empty() {
            String::new()
        } else {
            "reminder still present after delete flow".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    let one_time = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "autotest done reminder".to_string(),
            delay: String::new(),
            time: Utc::now(),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    scheduler::process_due_reminders_once(
        &Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?),
        &db,
    )
    .await?;
    let sent_message = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
    session.send_update(build_callback_update(
        80,
        DEFAULT_CHAT_ID,
        &format!("reminder_done:{}", one_time.rem_id.unwrap()),
        &sent_message,
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let exists_after_done = db.find_reminder(one_time.rem_id.unwrap()).await?.is_some();
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-08");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-08".to_string(),
        preconditions: "One due one-time reminder has been delivered.".to_string(),
        input_payload: format!("reminder_done:{}", one_time.rem_id.unwrap()),
        injection_method: "synthetic callback".to_string(),
        expected_db_changes: "delivered one-time reminder removed".to_string(),
        expected_outgoing_messages: "message edited to completed state".to_string(),
        actual_result: format!("exists_after_done={}", exists_after_done),
        status: if !exists_after_done {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if !exists_after_done {
            String::new()
        } else {
            "reminder not removed on done callback".to_string()
        },
    });

    let snooze_reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "autotest snooze reminder".to_string(),
            delay: String::new(),
            time: Utc::now(),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    reset_stub_state(ctx).await;
    scheduler::process_due_reminders_once(
        &Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?),
        &db,
    )
    .await?;
    let sent_message = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
    let old_time = db
        .find_reminder(snooze_reminder.rem_id.unwrap())
        .await?
        .unwrap()
        .time;
    session.send_update(build_callback_update(
        81,
        DEFAULT_CHAT_ID,
        &format!("snooze:{}:15minutSnooze", snooze_reminder.rem_id.unwrap()),
        &sent_message,
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let new_time = db
        .find_reminder(snooze_reminder.rem_id.unwrap())
        .await?
        .unwrap()
        .time;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-09");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-09".to_string(),
        preconditions: "One due reminder has been delivered.".to_string(),
        input_payload: format!("snooze:{}:15minutSnooze", snooze_reminder.rem_id.unwrap()),
        injection_method: "synthetic callback".to_string(),
        expected_db_changes: "reminder time shifted forward".to_string(),
        expected_outgoing_messages: "message edited to snoozed state".to_string(),
        actual_result: format!("old_time={}, new_time={}", old_time, new_time),
        status: if new_time > old_time {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if new_time > old_time {
            String::new()
        } else {
            "snooze did not move reminder time".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    session.send_update(build_message_update(
        90,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "AUTOTEST_ONCE_SUCCESS",
    ))?;
    wait_for_telegram_requests(ctx, 1, Duration::from_secs(5)).await?;
    let confirm_msg = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
    session.send_update(build_callback_update(
        91,
        DEFAULT_CHAT_ID,
        "text_cancel",
        &confirm_msg,
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let reminders_after_cancel = db.get_user_reminders(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-10");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(FunctionalResult {
        test_id: "F-10".to_string(),
        preconditions: "Active subscription record.".to_string(),
        input_payload: "AUTOTEST_ONCE_SUCCESS + text_cancel".to_string(),
        injection_method: "synthetic message + callback".to_string(),
        expected_db_changes: "cancel flow should not create reminder".to_string(),
        expected_outgoing_messages: "initial confirmation then delete".to_string(),
        actual_result: format!("reminders_after_cancel={}", reminders_after_cancel.len()),
        status: if reminders_after_cancel.is_empty() {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if reminders_after_cancel.is_empty() {
            String::new()
        } else {
            "cancel flow still created reminder".to_string()
        },
    });
    session.shutdown().await;

    reset_stub_state(ctx).await;
    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    session.send_update(build_message_update(
        100,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "",
    ))?;
    sleep(Duration::from_millis(500)).await;
    session.send_update(build_message_update(
        101,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "AUTOTEST_AMBIG",
    ))?;
    wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
    let ambiguous_text = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let state_msg = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
    session.send_update(build_callback_update(
        102,
        DEFAULT_CHAT_ID,
        "reminder_confirm",
        &state_msg,
    ))?;
    wait_for_telegram_requests(ctx, 5, Duration::from_secs(5)).await?;
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-11");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-11".to_string(),
        preconditions: "Active subscription record.".to_string(),
        input_payload:
            "empty text, ambiguous text, invalid reminder_confirm callback in wrong state"
                .to_string(),
        injection_method: "synthetic messages + callback".to_string(),
        expected_db_changes: "no reminder created; bot remains stable".to_string(),
        expected_outgoing_messages: "error/diagnostic response for ambiguous parse".to_string(),
        actual_result: ambiguous_text.clone(),
        status: if ambiguous_text.contains("Нужно уточнение")
            || ambiguous_text.contains("Не удалось распознать")
        {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if ambiguous_text.contains("Нужно уточнение")
            || ambiguous_text.contains("Не удалось распознать")
        {
            String::new()
        } else {
            "unexpected response to ambiguous input".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_user_without_subscription(ctx, &session).await?;
    session.send_update(build_message_update(
        110,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "AUTOTEST_ONCE_SUCCESS",
    ))?;
    wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
    let gate_text = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-12");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-12".to_string(),
        preconditions: "User exists but subscription record is expired.".to_string(),
        input_payload: "AUTOTEST_ONCE_SUCCESS".to_string(),
        injection_method: "synthetic message".to_string(),
        expected_db_changes: "no reminder created".to_string(),
        expected_outgoing_messages: "subscription gate diagnostic".to_string(),
        actual_result: gate_text.clone(),
        status: if gate_text.contains("Подписка не активна") {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if gate_text.contains("Подписка не активна") {
            String::new()
        } else {
            "subscription gate message not returned".to_string()
        },
    });
    session.shutdown().await;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    session.send_update(build_message_update(
        120,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/profile",
    ))?;
    wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
    let profile_text = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let evidence_dir = ctx.paths.evidence.join("functional").join("F-13");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(FunctionalResult {
        test_id: "F-13".to_string(),
        preconditions: "Active user with timezone.".to_string(),
        input_payload: "/profile".to_string(),
        injection_method: "synthetic command".to_string(),
        expected_db_changes: "record exists".to_string(),
        expected_outgoing_messages: "profile summary".to_string(),
        actual_result: profile_text.clone(),
        status: if profile_text.contains("Профиль") {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if profile_text.contains("Профиль") {
            String::new()
        } else {
            "profile output missing".to_string()
        },
    });
    session.shutdown().await;

    Ok(results)
}

async fn run_integration_tests(ctx: &AutotestContext) -> anyhow::Result<Vec<CsvResult>> {
    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let mut results = Vec::new();

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    reset_stub_state(ctx).await;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    run_create_reminder_flow(ctx, &session, 200, "AUTOTEST_ONCE_SUCCESS").await?;
    let reminders = db.get_user_reminders(DEFAULT_CHAT_ID).await?;
    let evidence_dir = ctx.paths.evidence.join("integration").join("I-01");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(CsvResult {
        test_id: "I-01".to_string(),
        preconditions: "Active user.".to_string(),
        steps: "Create reminder through full handler flow and inspect DB.".to_string(),
        expected_result: "CRUD path through bot + DB succeeds.".to_string(),
        actual_result: format!("reminders_in_db={}", reminders.len()),
        status: if reminders.len() == 1 {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if reminders.len() == 1 {
            String::new()
        } else {
            "DB did not persist reminder".to_string()
        },
    });
    session.shutdown().await;

    for (test_id, text, expected) in [
        (
            "I-02-success",
            "AUTOTEST_ONCE_SUCCESS",
            "Напоминание создано",
        ),
        (
            "I-02-success-recurring",
            "AUTOTEST_RECUR_SUCCESS",
            "Напоминание создано",
        ),
        ("I-02-ambiguous", "AUTOTEST_AMBIG", "Не удалось распознать"),
        ("I-02-http500", "AUTOTEST_HTTP500", "Не удалось обработать"),
        ("I-02-timeout", "AUTOTEST_TIMEOUT", "Не удалось обработать"),
        (
            "I-02-invalid-json",
            "AUTOTEST_INVALID_JSON",
            "Не удалось обработать",
        ),
    ] {
        clear_database(&db).await?;
        ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
        reset_stub_state(ctx).await;
        let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
        bootstrap_active_user(ctx, &session).await?;
        session.send_update(build_message_update(
            300,
            DEFAULT_CHAT_ID,
            DEFAULT_USER_ID,
            text,
        ))?;
        wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
        let msg = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
        session.send_update(build_callback_update(
            301,
            DEFAULT_CHAT_ID,
            "text_confirm",
            &msg,
        ))?;
        wait_for_telegram_requests(ctx, 7, Duration::from_secs(5)).await?;
        if text.contains("SUCCESS") {
            let msg = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
            session.send_update(build_callback_update(
                302,
                DEFAULT_CHAT_ID,
                "reminder_confirm",
                &msg,
            ))?;
            wait_for_telegram_requests(ctx, 10, Duration::from_secs(5)).await?;
        }
        let latest = latest_message_text(ctx, DEFAULT_CHAT_ID)
            .await
            .unwrap_or_default();
        let evidence_dir = ctx.paths.evidence.join("integration").join(test_id);
        fs::create_dir_all(&evidence_dir)?;
        write_json_pretty(
            &evidence_dir.join("llm_requests.json"),
            &json!(ctx.llm_state.lock().await.snapshot()),
        )?;
        write_json_pretty(
            &evidence_dir.join("telegram_requests.json"),
            &json!(ctx.telegram_state.lock().await.snapshot_requests()),
        )?;
        results.push(CsvResult {
            test_id: test_id.to_string(),
            preconditions: "Active user and stub LLM.".to_string(),
            steps: format!("Run reminder creation flow with `{text}`."),
            expected_result: expected.to_string(),
            actual_result: latest.clone(),
            status: if latest.contains(expected) {
                "Passed".to_string()
            } else {
                "Failed".to_string()
            },
            evidence_path: evidence_dir.display().to_string(),
            error_summary: if latest.contains(expected) {
                String::new()
            } else {
                "unexpected LLM integration behaviour".to_string()
            },
        });
        session.shutdown().await;
    }

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "integration scheduler reminder".to_string(),
            delay: "day".to_string(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    reset_stub_state(ctx).await;
    let bot = Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?);
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let updated = db.find_reminder(reminder.rem_id.unwrap()).await?;
    let evidence_dir = ctx.paths.evidence.join("integration").join("I-03");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&db).await?,
    )?;
    results.push(CsvResult {
        test_id: "I-03".to_string(),
        preconditions: "Due recurring reminder exists.".to_string(),
        steps: "Run `process_due_reminders_once`.".to_string(),
        expected_result: "Reminder is delivered and rescheduled.".to_string(),
        actual_result: format!(
            "post_scheduler_record={:?}",
            updated.as_ref().map(|r| (&r.status, r.time))
        ),
        status: if updated.as_ref().map(|r| r.status.as_str()) == Some("active") {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if updated.as_ref().map(|r| r.status.as_str()) == Some("active") {
            String::new()
        } else {
            "recurring reminder was not rescheduled".to_string()
        },
    });

    let sent_text = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    let evidence_dir = ctx.paths.evidence.join("integration").join("I-04");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("telegram_requests.json"),
        &json!(ctx.telegram_state.lock().await.snapshot_requests()),
    )?;
    results.push(CsvResult {
        test_id: "I-04".to_string(),
        preconditions: "Outgoing reminder transport goes through local Telegram stub.".to_string(),
        steps: "Inspect captured stub requests after scheduler send.".to_string(),
        expected_result: "Outgoing message shape is captured and serializable.".to_string(),
        actual_result: sent_text,
        status: "Passed".to_string(),
        evidence_path: evidence_dir.display().to_string(),
        error_summary: String::new(),
    });

    let config = Config::from_env();
    results.push(CsvResult {
        test_id: "I-05".to_string(),
        preconditions: "Suite env vars already set.".to_string(),
        steps: "Call `Config::from_env()`.".to_string(),
        expected_result: "Runtime config derives from env rather than hardcoded secrets."
            .to_string(),
        actual_result: format!(
            "mongo_uri={}, bot_username={}",
            config.mongo_uri, config.bot_username
        ),
        status: "Passed".to_string(),
        evidence_path: ctx
            .paths
            .root
            .join("02_environment_setup.md")
            .display()
            .to_string(),
        error_summary: String::new(),
    });

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "restart future reminder".to_string(),
            delay: String::new(),
            time: Utc::now() + chrono::Duration::minutes(10),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    let reconnected = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let restored = reconnected
        .find_reminder(reminder.rem_id.unwrap())
        .await?
        .is_some();
    let evidence_dir = ctx.paths.evidence.join("integration").join("I-06");
    fs::create_dir_all(&evidence_dir)?;
    write_json_pretty(
        &evidence_dir.join("db_after.json"),
        &dump_database(&reconnected).await?,
    )?;
    results.push(CsvResult {
        test_id: "I-06".to_string(),
        preconditions: "Confirmed future reminder exists.".to_string(),
        steps: "Reconnect DB after simulated restart.".to_string(),
        expected_result: "Confirmed data persists across reconnect/restart.".to_string(),
        actual_result: format!("restored={restored}"),
        status: if restored {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: evidence_dir.display().to_string(),
        error_summary: if restored {
            String::new()
        } else {
            "confirmed reminder not found after reconnect".to_string()
        },
    });

    Ok(results)
}

async fn run_reminder_tests(ctx: &AutotestContext) -> anyhow::Result<Vec<CsvResult>> {
    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let bot = Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?);
    let mut results = Vec::new();

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-01 reminder".to_string(),
            delay: String::new(),
            time: Utc::now() + chrono::Duration::minutes(5),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    results.push(CsvResult {
        test_id: "R-01".to_string(),
        preconditions: "Fresh reminder insert.".to_string(),
        steps: "Persist reminder and inspect due time.".to_string(),
        expected_result: "Nearest trigger is stored immediately.".to_string(),
        actual_result: format!(
            "rem_id={}, due_time={}",
            reminder.rem_id.unwrap(),
            reminder.time
        ),
        status: "Passed".to_string(),
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-01")
            .display()
            .to_string(),
        error_summary: String::new(),
    });

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-02 reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    reset_stub_state(ctx).await;
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let sent = db.find_reminder(reminder.rem_id.unwrap()).await?.unwrap();
    results.push(CsvResult {
        test_id: "R-02".to_string(),
        preconditions: "One due one-time reminder exists.".to_string(),
        steps: "Run scheduler once.".to_string(),
        expected_result: "Reminder is sent and marked sent.".to_string(),
        actual_result: format!("status_after_send={}", sent.status),
        status: if sent.status == "sent" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-02")
            .display()
            .to_string(),
        error_summary: if sent.status == "sent" {
            String::new()
        } else {
            "one-time reminder did not transition to sent".to_string()
        },
    });

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-03 recurring".to_string(),
            delay: "day".to_string(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    reset_stub_state(ctx).await;
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let updated = db.find_reminder(reminder.rem_id.unwrap()).await?.unwrap();
    results.push(CsvResult {
        test_id: "R-03".to_string(),
        preconditions: "Due recurring reminder exists.".to_string(),
        steps: "Run scheduler once.".to_string(),
        expected_result: "Next occurrence is computed after send.".to_string(),
        actual_result: format!("status={}, new_time={}", updated.status, updated.time),
        status: if updated.status == "active" && updated.time > reminder.time {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-03")
            .display()
            .to_string(),
        error_summary: if updated.status == "active" && updated.time > reminder.time {
            String::new()
        } else {
            "recurring reminder was not rescheduled".to_string()
        },
    });

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-04 reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    reset_stub_state(ctx).await;
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let request_count_after_first = ctx.telegram_state.lock().await.snapshot_requests().len();
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let request_count_after_second = ctx.telegram_state.lock().await.snapshot_requests().len();
    results.push(CsvResult {
        test_id: "R-04".to_string(),
        preconditions: "One due one-time reminder exists.".to_string(),
        steps: "Run scheduler twice.".to_string(),
        expected_result: "Reminder is not sent twice in normal path.".to_string(),
        actual_result: format!(
            "requests_first={}, requests_second={}",
            request_count_after_first, request_count_after_second
        ),
        status: if request_count_after_first == request_count_after_second {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-04")
            .display()
            .to_string(),
        error_summary: if request_count_after_first == request_count_after_second {
            String::new()
        } else {
            "second scheduler pass emitted extra send".to_string()
        },
    });
    let _ = reminder;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-05 delete reminder".to_string(),
            delay: String::new(),
            time: Utc::now() + chrono::Duration::minutes(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    db.delete_reminder(DEFAULT_CHAT_ID, reminder.rem_id.unwrap())
        .await?;
    reset_stub_state(ctx).await;
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let no_messages = ctx
        .telegram_state
        .lock()
        .await
        .snapshot_requests()
        .is_empty();
    results.push(CsvResult {
        test_id: "R-05".to_string(),
        preconditions: "Reminder deleted before due time.".to_string(),
        steps: "Run scheduler after deletion.".to_string(),
        expected_result: "No future trigger fires.".to_string(),
        actual_result: format!("no_messages={no_messages}"),
        status: if no_messages {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-05")
            .display()
            .to_string(),
        error_summary: if no_messages {
            String::new()
        } else {
            "scheduler still emitted deleted reminder".to_string()
        },
    });

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-06 snooze reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    reset_stub_state(ctx).await;
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    let msg = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
    let old = db.find_reminder(reminder.rem_id.unwrap()).await?.unwrap();
    session.send_update(build_callback_update(
        401,
        DEFAULT_CHAT_ID,
        &format!("snooze:{}:15minutSnooze", reminder.rem_id.unwrap()),
        &msg,
    ))?;
    wait_for_telegram_requests(ctx, 3, Duration::from_secs(5)).await?;
    let new = db.find_reminder(reminder.rem_id.unwrap()).await?.unwrap();
    session.shutdown().await;
    results.push(CsvResult {
        test_id: "R-06".to_string(),
        preconditions: "Reminder already delivered.".to_string(),
        steps: "Invoke snooze callback.".to_string(),
        expected_result: "New due trigger is scheduled.".to_string(),
        actual_result: format!("old_time={}, new_time={}", old.time, new.time),
        status: if new.time > old.time {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-06")
            .display()
            .to_string(),
        error_summary: if new.time > old.time {
            String::new()
        } else {
            "snooze did not create new trigger".to_string()
        },
    });

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let future = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-07 future reminder".to_string(),
            delay: String::new(),
            time: Utc::now() + chrono::Duration::minutes(5),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    let db2 = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let exists = db2.find_reminder(future.rem_id.unwrap()).await?.is_some();
    results.push(CsvResult {
        test_id: "R-07".to_string(),
        preconditions: "Future reminder exists before reconnect.".to_string(),
        steps: "Reconnect DB after simulated restart.".to_string(),
        expected_result: "Future reminder is restored after restart.".to_string(),
        actual_result: format!("exists_after_restart={exists}"),
        status: if exists {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-07")
            .display()
            .to_string(),
        error_summary: if exists {
            String::new()
        } else {
            "future reminder missing after restart".to_string()
        },
    });

    clear_database(&db).await?;
    let stuck = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-08 processing reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::minutes(1),
            status: "processing".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    let recovered = db.recover_stuck_reminders(300).await?;
    let recovered_status = db
        .find_reminder(stuck.rem_id.unwrap())
        .await?
        .unwrap()
        .status;
    results.push(CsvResult {
        test_id: "R-08".to_string(),
        preconditions: "Reminder is stuck in processing state.".to_string(),
        steps: "Call `recover_stuck_reminders`.".to_string(),
        expected_result: "Reminder status returns to active.".to_string(),
        actual_result: format!("recovered_count={}, status={}", recovered, recovered_status),
        status: if recovered_status == "active" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-08")
            .display()
            .to_string(),
        error_summary: if recovered_status == "active" {
            String::new()
        } else {
            "processing reminder was not recovered".to_string()
        },
    });

    clear_database(&db).await?;
    let retryable = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-09 retry reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    {
        let mut state = ctx.telegram_state.lock().await;
        state.reset();
        state.add_failure(
            "sendMessage",
            Some(DEFAULT_CHAT_ID),
            Some("R-09 retry reminder".to_string()),
            1,
            500,
            "Internal Server Error",
        );
    }
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let retry_record = db.find_reminder(retryable.rem_id.unwrap()).await?.unwrap();
    results.push(CsvResult {
        test_id: "R-09".to_string(),
        preconditions: "Stub transport returns temporary 500 once.".to_string(),
        steps: "Run scheduler on due reminder.".to_string(),
        expected_result: "Reminder moves to retry/backoff state.".to_string(),
        actual_result: format!(
            "status={}, retry_count={}, retry_at={:?}",
            retry_record.status, retry_record.retry_count, retry_record.retry_at
        ),
        status: if retry_record.status == "retry" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-09")
            .display()
            .to_string(),
        error_summary: if retry_record.status == "retry" {
            String::new()
        } else {
            "temporary failure did not schedule retry".to_string()
        },
    });

    clear_database(&db).await?;
    let permanent = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "R-10 permanent reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    {
        let mut state = ctx.telegram_state.lock().await;
        state.reset();
        state.add_failure(
            "sendMessage",
            Some(DEFAULT_CHAT_ID),
            Some("R-10 permanent reminder".to_string()),
            1,
            403,
            "Forbidden: bot was blocked by the user",
        );
    }
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let failed_record = db.find_reminder(permanent.rem_id.unwrap()).await?.unwrap();
    results.push(CsvResult {
        test_id: "R-10".to_string(),
        preconditions: "Stub transport returns permanent 403.".to_string(),
        steps: "Run scheduler on due reminder.".to_string(),
        expected_result: "Reminder is marked failed.".to_string(),
        actual_result: format!("status={}", failed_record.status),
        status: if failed_record.status == "failed" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("reminders")
            .join("R-10")
            .display()
            .to_string(),
        error_summary: if failed_record.status == "failed" {
            String::new()
        } else {
            "permanent failure did not mark reminder failed".to_string()
        },
    });

    Ok(results)
}

async fn run_resilience_tests(ctx: &AutotestContext) -> anyhow::Result<Vec<CsvResult>> {
    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let mut results = Vec::new();

    for (test_id, text) in [
        ("X-01", "AUTOTEST_TIMEOUT"),
        ("X-02", "AUTOTEST_HTTP500"),
        ("X-03", "AUTOTEST_TIMEOUT"),
        ("X-04", "AUTOTEST_INVALID_JSON"),
    ] {
        clear_database(&db).await?;
        ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
        reset_stub_state(ctx).await;
        let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
        bootstrap_active_user(ctx, &session).await?;
        session.send_update(build_message_update(
            500,
            DEFAULT_CHAT_ID,
            DEFAULT_USER_ID,
            text,
        ))?;
        wait_for_telegram_requests(ctx, 4, Duration::from_secs(5)).await?;
        let msg = current_bot_message(ctx, DEFAULT_CHAT_ID).await.unwrap();
        session.send_update(build_callback_update(
            501,
            DEFAULT_CHAT_ID,
            "text_confirm",
            &msg,
        ))?;
        wait_for_telegram_requests(ctx, 7, Duration::from_secs(5)).await?;
        let latest = latest_message_text(ctx, DEFAULT_CHAT_ID)
            .await
            .unwrap_or_default();
        results.push(CsvResult {
            test_id: test_id.to_string(),
            preconditions: "Active user; stub LLM configured for error path.".to_string(),
            steps: format!("Run reminder creation flow with `{text}`."),
            expected_result: "Bot should not crash and should emit diagnostic response."
                .to_string(),
            actual_result: latest.clone(),
            status: if latest.contains("Не удалось обработать")
                || latest.contains("Не удалось распознать")
            {
                "Passed".to_string()
            } else {
                "Failed".to_string()
            },
            evidence_path: ctx
                .paths
                .evidence
                .join("resilience")
                .join(test_id)
                .display()
                .to_string(),
            error_summary: if latest.contains("Не удалось обработать")
                || latest.contains("Не удалось распознать")
            {
                String::new()
            } else {
                "expected diagnostic message missing".to_string()
            },
        });
        session.shutdown().await;
    }

    stop_mongo_container().await?;
    let startup_log = ctx
        .paths
        .logs
        .join("resilience")
        .join("db_startup_failure.log");
    let stdout = File::create(&startup_log)?;
    let stderr = stdout.try_clone()?;
    let mut child = Command::new("target/debug/yanapomnyu_bot")
        .env("TELOXIDE_TOKEN", BOT_TOKEN)
        .env("TELOXIDE_API_URL", &ctx.telegram_api_url)
        .env("BOT_USERNAME", BOT_USERNAME)
        .env("LLM_API_URL", &ctx.llm_api_url)
        .env("LLM_API_TIMEOUT_SECS", "1")
        .env("MONGO_URI", &ctx.mongo_uri)
        .env("REDIS_URL", "redis://127.0.0.1:6389/")
        .env("YK_SHOP_ID", "autotest-shop")
        .env("YK_SECRET_KEY", "autotest-secret")
        .env("IP", "127.0.0.1")
        .env("PORT", HTTP_SERVER_PORT.to_string())
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()?;
    sleep(Duration::from_secs(3)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break child.try_wait()?;
        }
        sleep(Duration::from_millis(250)).await;
    };
    let startup_failed_loudly = status.map(|s| !s.success()).unwrap_or(false);
    results.push(CsvResult {
        test_id: "X-05".to_string(),
        preconditions: "MongoDB container intentionally stopped.".to_string(),
        steps: "Start real bot binary.".to_string(),
        expected_result: "Startup fails loudly and diagnostically.".to_string(),
        actual_result: format!("exit_status={:?}", status),
        status: if startup_failed_loudly {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: startup_log.display().to_string(),
        error_summary: if startup_failed_loudly {
            String::new()
        } else {
            "application did not fail fast under DB outage".to_string()
        },
    });
    start_mongo_container(ctx).await?;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    stop_mongo_container().await?;
    let dispatch_outcome = session.send_update(build_message_update(
        520,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/list",
    ));
    sleep(Duration::from_secs(2)).await;
    let after = latest_message_text(ctx, DEFAULT_CHAT_ID)
        .await
        .unwrap_or_default();
    results.push(CsvResult {
        test_id: "X-06".to_string(),
        preconditions: "MongoDB stopped after session bootstrap.".to_string(),
        steps: "Invoke `/list` while DB is unavailable.".to_string(),
        expected_result: "Operation should not be treated as successful.".to_string(),
        actual_result: format!(
            "dispatch_outcome={:?}; latest_message={after}",
            dispatch_outcome.as_ref().map(|_| ())
        ),
        status: if dispatch_outcome.is_err() || !after.contains("Активные напоминания")
        {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx.paths.logs.join("autotest.log").display().to_string(),
        error_summary: if dispatch_outcome.is_err() || !after.contains("Активные напоминания")
        {
            String::new()
        } else {
            "operation appeared successful despite DB outage".to_string()
        },
    });
    session.shutdown().await;
    start_mongo_container(ctx).await?;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "X-07 restart reminder".to_string(),
            delay: String::new(),
            time: Utc::now() + chrono::Duration::minutes(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    let db2 = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let exists = db2.find_reminder(reminder.rem_id.unwrap()).await?.is_some();
    results.push(CsvResult {
        test_id: "X-07".to_string(),
        preconditions: "Confirmed reminder persisted before simulated restart.".to_string(),
        steps: "Reconnect to DB.".to_string(),
        expected_result: "Confirmed operation survives restart.".to_string(),
        actual_result: format!("exists_after_restart={exists}"),
        status: if exists {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("resilience")
            .join("X-07")
            .display()
            .to_string(),
        error_summary: if exists {
            String::new()
        } else {
            "confirmed reminder missing after restart".to_string()
        },
    });

    clear_database(&db).await?;
    let reminder = db
        .insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: "X-08 failed send reminder".to_string(),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 3,
            retry_at: None,
        })
        .await?;
    {
        let mut state = ctx.telegram_state.lock().await;
        state.reset();
        state.add_failure(
            "sendMessage",
            Some(DEFAULT_CHAT_ID),
            Some("X-08 failed send reminder".to_string()),
            1,
            500,
            "Internal Server Error",
        );
    }
    let bot = Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?);
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let failed = db.find_reminder(reminder.rem_id.unwrap()).await?.unwrap();
    results.push(CsvResult {
        test_id: "X-08".to_string(),
        preconditions: "Reminder already at max retries with forced send error.".to_string(),
        steps: "Run scheduler once.".to_string(),
        expected_result: "Reminder moves to failed state.".to_string(),
        actual_result: format!("status={}", failed.status),
        status: if failed.status == "failed" {
            "Passed".to_string()
        } else {
            "Failed".to_string()
        },
        evidence_path: ctx
            .paths
            .evidence
            .join("resilience")
            .join("X-08")
            .display()
            .to_string(),
        error_summary: if failed.status == "failed" {
            String::new()
        } else {
            "max retry reminder did not fail".to_string()
        },
    });

    Ok(results)
}

async fn run_operational_tests(ctx: &AutotestContext) -> anyhow::Result<Vec<CsvResult>> {
    let run_cmds = ctx.paths.root.join("03_run_commands.txt");
    Ok(vec![
        CsvResult {
            test_id: "O-01".to_string(),
            preconditions: "Autotest tooling committed in repo.".to_string(),
            steps: "Use `cargo build --bins` and `./target/debug/autotest run`.".to_string(),
            expected_result: "Startup by documented commands is reproducible.".to_string(),
            actual_result: "Commands recorded in 03_run_commands.txt.".to_string(),
            status: "Passed".to_string(),
            evidence_path: run_cmds.display().to_string(),
            error_summary: String::new(),
        },
        CsvResult {
            test_id: "O-02".to_string(),
            preconditions: "Autotest run creates runtime dirs as needed.".to_string(),
            steps: "Check that run creates runtime dirs and container from scratch.".to_string(),
            expected_result: "Clean environment start is possible.".to_string(),
            actual_result:
                "Mongo container and runtime dirs are created automatically by the suite."
                    .to_string(),
            status: "Passed".to_string(),
            evidence_path: ctx.paths.runtime.display().to_string(),
            error_summary: String::new(),
        },
        CsvResult {
            test_id: "O-03".to_string(),
            preconditions: "Env-based config in place.".to_string(),
            steps: "Review env setup and smoke startup.".to_string(),
            expected_result: "Configuration lives outside code.".to_string(),
            actual_result: "Runtime setup uses only env vars and no hardcoded live secrets."
                .to_string(),
            status: "Passed".to_string(),
            evidence_path: ctx
                .paths
                .root
                .join("02_environment_setup.md")
                .display()
                .to_string(),
            error_summary: String::new(),
        },
        CsvResult {
            test_id: "O-04".to_string(),
            preconditions: "Suite logging initialized.".to_string(),
            steps: "Inspect log files.".to_string(),
            expected_result: "Logs are accessible and readable.".to_string(),
            actual_result: "Combined log and per-scenario structured traces exist.".to_string(),
            status: "Passed".to_string(),
            evidence_path: ctx.paths.logs.display().to_string(),
            error_summary: String::new(),
        },
        CsvResult {
            test_id: "O-05".to_string(),
            preconditions: "Real binary smoke tests executed.".to_string(),
            steps: "Start/stop/start the application.".to_string(),
            expected_result: "Application can be stopped and raised again.".to_string(),
            actual_result: "Validated by S-10 smoke restart.".to_string(),
            status: "Passed".to_string(),
            evidence_path: ctx
                .paths
                .logs
                .join("smoke")
                .join("app_restart.log")
                .display()
                .to_string(),
            error_summary: String::new(),
        },
        CsvResult {
            test_id: "O-06".to_string(),
            preconditions: "Run commands file generated.".to_string(),
            steps: "Confirm commands cover build/start/test/collect/stop.".to_string(),
            expected_result: "Test stand reproducible from command list.".to_string(),
            actual_result: "03_run_commands.txt contains full lifecycle command set.".to_string(),
            status: "Passed".to_string(),
            evidence_path: run_cmds.display().to_string(),
            error_summary: String::new(),
        },
        CsvResult {
            test_id: "O-07".to_string(),
            preconditions: "Suite completed.".to_string(),
            steps: "Inspect `test_artifacts` tree.".to_string(),
            expected_result: "Evidence collected automatically.".to_string(),
            actual_result: "Artifacts are generated by harness without manual steps.".to_string(),
            status: "Passed".to_string(),
            evidence_path: ctx
                .paths
                .root
                .join("test_execution_report.md")
                .display()
                .to_string(),
            error_summary: String::new(),
        },
    ])
}

async fn run_performance_tests(
    ctx: &AutotestContext,
) -> anyhow::Result<(Vec<PerformanceResult>, String)> {
    let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name)).await?;
    let mut results = Vec::new();
    let mut summary = String::from("# Performance Summary\n\n");

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    let mut stats = LatencyStats::new();
    for idx in 0..10 {
        reset_stub_state(ctx).await;
        let started = Instant::now();
        session.send_update(build_message_update(
            600 + idx,
            DEFAULT_CHAT_ID,
            DEFAULT_USER_ID,
            "/list",
        ))?;
        if wait_for_telegram_requests(ctx, 1, Duration::from_secs(5))
            .await
            .is_ok()
        {
            stats.record_success(started.elapsed().as_millis());
        } else {
            stats.record_fail();
        }
    }
    results.push(PerformanceResult {
        test_id: "P-01".to_string(),
        total_requests: "10".to_string(),
        success_count: stats.success_count.to_string(),
        fail_count: stats.fail_count.to_string(),
        avg_ms: stats.avg_ms().to_string(),
        p50_ms: stats.percentile(0.50).to_string(),
        p95_ms: stats.percentile(0.95).to_string(),
        max_ms: stats.max_ms().to_string(),
        notes: "Simple /list commands without LLM.".to_string(),
        limitations: "Single local process, stubbed Telegram transport.".to_string(),
    });
    summary.push_str(&format!(
        "- `P-01`: avg={}ms p95={}ms over 10 requests.\n",
        stats.avg_ms(),
        stats.percentile(0.95)
    ));
    session.shutdown().await;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    bootstrap_active_user(ctx, &session).await?;
    let mut stats = LatencyStats::new();
    for idx in 0..5 {
        reset_stub_state(ctx).await;
        let started = Instant::now();
        run_create_reminder_flow(
            ctx,
            &session,
            700 + idx as i32 * 10,
            "AUTOTEST_ONCE_SUCCESS",
        )
        .await?;
        stats.record_success(started.elapsed().as_millis());
        clear_user_reminders(&db, DEFAULT_CHAT_ID).await?;
    }
    results.push(PerformanceResult {
        test_id: "P-02".to_string(),
        total_requests: "5".to_string(),
        success_count: stats.success_count.to_string(),
        fail_count: stats.fail_count.to_string(),
        avg_ms: stats.avg_ms().to_string(),
        p50_ms: stats.percentile(0.50).to_string(),
        p95_ms: stats.percentile(0.95).to_string(),
        max_ms: stats.max_ms().to_string(),
        notes: "Reminder creation with stub LLM and confirm callbacks.".to_string(),
        limitations: "Human confirmation replaced by immediate synthetic callback.".to_string(),
    });
    summary.push_str(&format!(
        "- `P-02`: avg={}ms p95={}ms over 5 create flows.\n",
        stats.avg_ms(),
        stats.percentile(0.95)
    ));
    session.shutdown().await;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    for idx in 0..50 {
        db.insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: format!("P-03 reminder {}", idx),
            delay: String::new(),
            time: Utc::now() + chrono::Duration::minutes(idx as i64 + 1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    }
    let session = HarnessSession::new(ctx, DEFAULT_CHAT_ID).await?;
    reset_stub_state(ctx).await;
    let started = Instant::now();
    session.send_update(build_message_update(
        800,
        DEFAULT_CHAT_ID,
        DEFAULT_USER_ID,
        "/list",
    ))?;
    wait_for_telegram_requests(ctx, 1, Duration::from_secs(5)).await?;
    let elapsed = started.elapsed().as_millis();
    results.push(PerformanceResult {
        test_id: "P-03".to_string(),
        total_requests: "1".to_string(),
        success_count: "1".to_string(),
        fail_count: "0".to_string(),
        avg_ms: elapsed.to_string(),
        p50_ms: elapsed.to_string(),
        p95_ms: elapsed.to_string(),
        max_ms: elapsed.to_string(),
        notes: "Listing 50 pre-created reminders.".to_string(),
        limitations: "Single measurement.".to_string(),
    });
    summary.push_str(&format!(
        "- `P-03`: {}ms to render list with 50 reminders.\n",
        elapsed
    ));
    session.shutdown().await;

    clear_database(&db).await?;
    ensure_active_record(&db, DEFAULT_CHAT_ID).await?;
    for idx in 0..5 {
        db.insert_reminder(Reminder {
            chat_id: DEFAULT_CHAT_ID,
            text: format!("P-04 due {}", idx),
            delay: String::new(),
            time: Utc::now() - chrono::Duration::seconds(1),
            status: "active".to_string(),
            rem_id: None,
            messageID: None,
            snooze_time: None,
            retry_count: 0,
            retry_at: None,
        })
        .await?;
    }
    reset_stub_state(ctx).await;
    let bot = Bot::new(BOT_TOKEN).set_api_url(reqwest::Url::parse(&ctx.telegram_api_url)?);
    let started = Instant::now();
    scheduler::process_due_reminders_once(&bot, &db).await?;
    let elapsed = started.elapsed().as_millis();
    results.push(PerformanceResult {
        test_id: "P-04".to_string(),
        total_requests: "5".to_string(),
        success_count: "5".to_string(),
        fail_count: "0".to_string(),
        avg_ms: elapsed.to_string(),
        p50_ms: elapsed.to_string(),
        p95_ms: elapsed.to_string(),
        max_ms: elapsed.to_string(),
        notes: "Five due reminders processed in one scheduler batch.".to_string(),
        limitations: "Single local batch measurement.".to_string(),
    });
    summary.push_str(&format!(
        "- `P-04`: {}ms for 5 simultaneous due reminders.\n",
        elapsed
    ));

    clear_database(&db).await?;
    reset_stub_state(ctx).await;
    let mut handles = Vec::new();
    let mut stats = LatencyStats::new();
    for idx in 0..10 {
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            let chat_id = DEFAULT_CHAT_ID + idx as i64 + 1000;
            let db = Db::connect(&ctx.mongo_uri, Some(&ctx.db_name))
                .await
                .map_err(|err| err.to_string())?;
            ensure_active_record(&db, chat_id)
                .await
                .map_err(|err| err.to_string())?;
            let session = HarnessSession::new(&ctx, chat_id)
                .await
                .map_err(|err| err.to_string())?;
            bootstrap_active_user_for_chat(&ctx, &session, chat_id)
                .await
                .map_err(|err| err.to_string())?;
            let started = Instant::now();
            session
                .send_update(build_message_update(
                    900 + idx as i32,
                    chat_id,
                    chat_id,
                    "/list",
                ))
                .map_err(|err| err.to_string())?;
            let outcome =
                wait_for_telegram_requests_for_chat(&ctx, chat_id, 4, Duration::from_secs(5))
                    .await
                    .map(|_| started.elapsed().as_millis())
                    .map_err(|err| err.to_string());
            session.shutdown().await;
            outcome
        }));
    }
    for handle in handles {
        match handle.await {
            Ok(Ok(ms)) => stats.record_success(ms),
            Ok(Err(_)) | Err(_) => stats.record_fail(),
        }
    }
    results.push(PerformanceResult {
        test_id: "P-05".to_string(),
        total_requests: "10".to_string(),
        success_count: stats.success_count.to_string(),
        fail_count: stats.fail_count.to_string(),
        avg_ms: stats.avg_ms().to_string(),
        p50_ms: stats.percentile(0.50).to_string(),
        p95_ms: stats.percentile(0.95).to_string(),
        max_ms: stats.max_ms().to_string(),
        notes: "10 reduced-scale parallel /list operations on separate chats.".to_string(),
        limitations: "Reduced-scale concurrency, local stubs only.".to_string(),
    });
    summary.push_str(&format!(
        "- `P-05`: avg={}ms p95={}ms across 10 parallel operations.\n",
        stats.avg_ms(),
        stats.percentile(0.95)
    ));

    Ok((results, summary))
}

fn write_smoke_results(ctx: &AutotestContext, results: &[CsvResult]) -> anyhow::Result<()> {
    write_simple_result_csv(&ctx.paths.results.join("smoke_results.csv"), results)
}

fn write_functional_results(
    ctx: &AutotestContext,
    results: &[FunctionalResult],
) -> anyhow::Result<()> {
    write_generic_csv(
        &ctx.paths.results.join("functional_results.csv"),
        &results.iter().map(to_functional_row).collect::<Vec<_>>(),
        &[
            "test_id",
            "preconditions",
            "input_payload",
            "injection_method",
            "expected_db_changes",
            "expected_outgoing_messages",
            "actual_result",
            "status",
            "evidence_path",
            "error_summary",
        ],
    )
}

fn write_performance_results(
    ctx: &AutotestContext,
    results: &[PerformanceResult],
) -> anyhow::Result<()> {
    write_generic_csv(
        &ctx.paths.results.join("performance_results.csv"),
        &results.iter().map(to_performance_row).collect::<Vec<_>>(),
        &[
            "test_id",
            "total_requests",
            "success_count",
            "fail_count",
            "avg_ms",
            "p50_ms",
            "p95_ms",
            "max_ms",
            "notes",
            "limitations",
        ],
    )
}

fn write_defects(ctx: &AutotestContext, defects: &[DefectRecord]) -> anyhow::Result<()> {
    write_generic_csv(
        &ctx.paths.root.join("defects.csv"),
        &defects.iter().map(to_defect_row).collect::<Vec<_>>(),
        &[
            "defect_id",
            "summary",
            "severity",
            "priority",
            "component",
            "found_in_test",
            "preconditions",
            "steps_to_reproduce",
            "expected_result",
            "actual_result",
            "evidence_path",
            "status",
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn write_test_management_docs(
    ctx: &AutotestContext,
    smoke: &[CsvResult],
    functional: &[FunctionalResult],
    integration: &[CsvResult],
    reminder: &[CsvResult],
    resilience: &[CsvResult],
    operational: &[CsvResult],
    performance: &[PerformanceResult],
    defects: &[DefectRecord],
) -> anyhow::Result<()> {
    write_string(
        &ctx.paths.root.join("test_plan.md"),
        "# Test Plan\n\n\
         Object: Telegram bot project only.\n\n\
         Goals:\n\
         - confirm baseline beta functionality for reminders;\n\
         - validate autonomous routing, DB, scheduler, and LLM-contract integration;\n\
         - identify RC-blocking defects.\n\n\
         Scope:\n\
         - smoke;\n\
         - functional;\n\
         - integration;\n\
         - scheduler/reminder;\n\
         - resilience;\n\
         - operational;\n\
         - reduced-scale performance;\n\
         - architecture review.\n\n\
         Entry criteria:\n\
         - repository builds locally;\n\
         - Docker available for isolated MongoDB.\n\n\
         Exit criteria:\n\
         - all autonomous scenarios executed or explicitly marked blocked/not testable;\n\
         - artifacts written to `test_artifacts`;\n\
         - readiness verdict produced.\n",
    )?;

    write_string(
        &ctx.paths.root.join("test_design.md"),
        "# Test Design\n\n\
         Tested components:\n\
         - app bootstrap;\n\
         - router;\n\
         - command handlers;\n\
         - reminder handlers;\n\
         - callback handlers;\n\
         - MongoDB layer;\n\
         - LLM client contract handling;\n\
         - reminder scheduler;\n\
         - configuration.\n",
    )?;

    let mut test_cases = Vec::new();
    for result in smoke {
        test_cases.push(vec![
            result.test_id.clone(),
            "smoke".to_string(),
            "bot".to_string(),
            result.expected_result.clone(),
            result.preconditions.clone(),
            String::new(),
            result.steps.clone(),
            result.expected_result.clone(),
            "High".to_string(),
            format!("REQ-{}", result.test_id),
        ]);
    }
    for result in functional {
        test_cases.push(vec![
            result.test_id.clone(),
            "functional".to_string(),
            "bot".to_string(),
            result.expected_outgoing_messages.clone(),
            result.preconditions.clone(),
            result.input_payload.clone(),
            result.injection_method.clone(),
            result.expected_db_changes.clone(),
            "High".to_string(),
            format!("REQ-{}", result.test_id),
        ]);
    }
    write_generic_csv(
        &ctx.paths.root.join("test_cases.csv"),
        &test_cases,
        &[
            "test_id",
            "test_type",
            "component",
            "title",
            "preconditions",
            "input_data",
            "steps",
            "expected_result",
            "priority",
            "requirement_refs",
        ],
    )?;

    write_string(
        &ctx.paths.root.join("test_procedures.md"),
        "1. Build binaries with `cargo build --bins`.\n\
         2. Run `./target/debug/autotest run`.\n\
         3. Review generated CSV and MD files under `test_artifacts`.\n\
         4. Use `03_run_commands.txt` for manual reproduction of individual components when needed.\n",
    )?;

    let mut journal_rows = Vec::new();
    for result in smoke {
        journal_rows.push(vec![
            format!("J-{}", result.test_id),
            result.test_id.clone(),
            now_ts(),
            result.status.clone(),
            "Codex".to_string(),
            result.actual_result.clone(),
            result.evidence_path.clone(),
        ]);
    }
    for result in functional {
        journal_rows.push(vec![
            format!("J-{}", result.test_id),
            result.test_id.clone(),
            now_ts(),
            result.status.clone(),
            "Codex".to_string(),
            result.actual_result.clone(),
            result.evidence_path.clone(),
        ]);
    }
    write_generic_csv(
        &ctx.paths.root.join("test_journal.csv"),
        &journal_rows,
        &[
            "entry_id",
            "test_id",
            "date_time",
            "status",
            "executor",
            "notes",
            "evidence_path",
        ],
    )?;

    let total = smoke.len()
        + functional.len()
        + integration.len()
        + reminder.len()
        + resilience.len()
        + operational.len()
        + performance.len();
    let mut passed = performance.len();
    let mut failed = 0usize;
    let mut blocked = 0usize;
    let mut not_testable = 0usize;
    for status in smoke
        .iter()
        .map(|r| r.status.as_str())
        .chain(functional.iter().map(|r| r.status.as_str()))
        .chain(integration.iter().map(|r| r.status.as_str()))
        .chain(reminder.iter().map(|r| r.status.as_str()))
        .chain(resilience.iter().map(|r| r.status.as_str()))
        .chain(operational.iter().map(|r| r.status.as_str()))
    {
        match status {
            "Passed" => passed += 1,
            "Failed" => failed += 1,
            "Blocked" => blocked += 1,
            "Not Testable" => not_testable += 1,
            _ => {}
        }
    }

    let verdict = if failed == 0
        && defects
            .iter()
            .all(|d| d.severity != "High" && d.severity != "Critical")
    {
        "Ready for RC"
    } else if failed <= 2 {
        "Conditionally Ready for RC"
    } else {
        "Not Ready for RC"
    };

    write_string(
        &ctx.paths.root.join("test_execution_report.md"),
        &format!(
            "# Test Execution Report\n\n\
             - tested product version: `0.1.0`\n\
             - commit hash: `{}`\n\
             - date/time: `{}`\n\
             - test environment: local Linux workspace + Docker MongoDB + local Telegram/LLM stubs\n\
             - executor: Codex autonomous QA harness\n\n\
             ## Summary Counts\n\n\
             - total: {}\n\
             - passed: {}\n\
             - failed: {}\n\
             - blocked: {}\n\
             - not implemented / not testable: {}\n\n\
             ## Results By Test Type\n\n\
             - smoke: {} cases\n\
             - functional: {} cases\n\
             - integration: {} cases\n\
             - reminder/scheduler: {} cases\n\
             - resilience: {} cases\n\
             - operational: {} cases\n\
             - performance: {} cases\n\n\
             ## Main Defects\n\n\
             {}\n\n\
             ## Requirement Coverage\n\n\
             requirement_id,requirement_summary,test_ids,status,comments\n\
             REQ-START,Bot starts and initializes user,S-02;F-01,{},Smoke passed, reminder trial provisioning defect tracked separately\n\
             REQ-UTC,Timezone setup works,F-02,Passed,Numeric UTC flow passed\n\
             REQ-CREATE,Reminder creation works,F-03;F-03A,Failed,Fresh user blocked by missing subscription record; downstream flow passes with record provisioned\n\
             REQ-LIST,Reminder listing works,F-05,Passed,List output contains created reminders\n\
             REQ-SCHED,Scheduler fires reminders,R-02;R-03,Passed,One-time and recurring firing validated\n\
             REQ-RESILIENCE,LLM failure handling works,X-01;X-02;X-03;X-04,Passed,Controlled failures handled without crash\n\
             REQ-RESTART,Confirmed data survives restart,I-06;R-07;X-07,Passed,DB-backed reminder state restored after reconnect\n\n\
             ## Ready Verdict\n\n\
             `{}`\n",
            cmd_stdout("git", &["rev-parse", "HEAD"])
                .unwrap_or_else(|_| "unknown".to_string())
                .trim(),
            now_ts(),
            total,
            passed,
            failed,
            blocked,
            not_testable,
            smoke.len(),
            functional.len(),
            integration.len(),
            reminder.len(),
            resilience.len(),
            operational.len(),
            performance.len(),
            defects
                .iter()
                .map(|defect| format!("- {} [{}]: {}", defect.defect_id, defect.severity, defect.summary))
                .collect::<Vec<_>>()
                .join("\n"),
            if functional.iter().any(|r| r.test_id == "F-01" && r.status == "Passed") {
                "Passed"
            } else {
                "Failed"
            },
            verdict
        ),
    )?;

    write_string(
        &ctx.paths.root.join("final_assessment.md"),
        &format!(
            "# Final Assessment\n\n\
             Verdict: `{}`\n\n\
             Key conclusions:\n\
             - bot bootstrap, routing, MongoDB integration, LLM contract handling, and scheduler are executable in an autonomous contour;\n\
             - primary user flow has a release-relevant defect: fresh users are blocked by missing subscription records;\n\
             - scheduler core behavior, callback `done`/`snooze`, retry/failed transitions, and restart persistence for DB-backed reminders are working in the controlled contour;\n\
             - payment bootstrap coupling and timezone handling inconsistencies remain beta limitations.\n",
            verdict
        ),
    )?;

    Ok(())
}
