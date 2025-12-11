//! API модуль - работа с внешними сервисами.
//!
//! ## Подмодули
//!
//! - [`db`] - MongoDB: пользователи, напоминания, платежи
//! - [`cache`] - Redis: кэширование pending платежей
//! - [`llm_client`] - HTTP клиент для LLM API сервиса
//! - [`llm_models`] - Модели данных для парсинга ответов LLM
//! - [`time_calculator`] - Вычисление DateTime из TimeSpec
//! - [`payments`] - YooKassa: создание и обработка платежей

pub mod cache;
pub mod db;
pub mod llm_client;
pub mod llm_models;
pub mod payments;
pub mod time_calculator;
