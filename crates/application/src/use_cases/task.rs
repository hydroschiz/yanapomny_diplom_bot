use chrono::{DateTime, Utc};
use domain::{Task, TaskId, TaskPriority, TaskStatus, UserId};

use crate::{
    ApplicationError, ApplicationResult, Clock, NaturalLanguageInterpreter, TaskRepository,
    UserRepository,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskCommand {
    pub user_id: UserId,
    pub title: String,
    pub description: Option<String>,
    pub priority: TaskPriority,
    pub due_at: Option<DateTime<Utc>>,
}

pub struct CreateTaskUseCase<'a, R, C> {
    tasks: &'a R,
    clock: &'a C,
}

impl<'a, R, C> CreateTaskUseCase<'a, R, C>
where
    R: TaskRepository,
    C: Clock,
{
    pub const fn new(tasks: &'a R, clock: &'a C) -> Self {
        Self { tasks, clock }
    }

    pub async fn execute(&self, command: CreateTaskCommand) -> ApplicationResult<Task> {
        let now = self.clock.now();
        let mut task = Task::new(command.user_id, command.title, now);
        task.priority = command.priority;
        task.due_at = command.due_at;
        task.description = command.description;
        self.tasks.create_task(task).await
    }
}

pub struct CreateTaskFromTextUseCase<'a, U, T, I, C> {
    users: &'a U,
    tasks: &'a T,
    interpreter: &'a I,
    clock: &'a C,
}

impl<'a, U, T, I, C> CreateTaskFromTextUseCase<'a, U, T, I, C>
where
    U: UserRepository,
    T: TaskRepository,
    I: NaturalLanguageInterpreter,
    C: Clock,
{
    pub const fn new(users: &'a U, tasks: &'a T, interpreter: &'a I, clock: &'a C) -> Self {
        Self {
            users,
            tasks,
            interpreter,
            clock,
        }
    }

    pub async fn execute(&self, user_id: UserId, text: &str) -> ApplicationResult<Task> {
        let user =
            self.users
                .find_user(user_id)
                .await?
                .ok_or_else(|| ApplicationError::NotFound {
                    entity: "user",
                    id: user_id.to_string(),
                })?;
        let interpreted = self.interpreter.interpret_task(text, &user).await?;
        let now = self.clock.now();
        let mut task = Task::new(user_id, interpreted.title, now);
        task.description = interpreted.description;
        task.due_at = Some(interpreted.trigger_at);
        self.tasks.create_task(task).await
    }
}

pub struct ListTasksUseCase<'a, R> {
    tasks: &'a R,
}

impl<'a, R> ListTasksUseCase<'a, R>
where
    R: TaskRepository,
{
    pub const fn new(tasks: &'a R) -> Self {
        Self { tasks }
    }

    pub async fn execute(&self, user_id: UserId) -> ApplicationResult<Vec<Task>> {
        self.tasks.list_tasks(user_id).await
    }
}

pub struct CompleteTaskUseCase<'a, R, C> {
    tasks: &'a R,
    clock: &'a C,
}

impl<'a, R, C> CompleteTaskUseCase<'a, R, C>
where
    R: TaskRepository,
    C: Clock,
{
    pub const fn new(tasks: &'a R, clock: &'a C) -> Self {
        Self { tasks, clock }
    }

    pub async fn execute(&self, task_id: TaskId) -> ApplicationResult<Task> {
        let mut task =
            self.tasks
                .find_task(task_id)
                .await?
                .ok_or_else(|| ApplicationError::NotFound {
                    entity: "task",
                    id: task_id.to_string(),
                })?;
        task.complete(self.clock.now())?;
        self.tasks.save_task(&task).await?;
        Ok(task)
    }
}

pub struct DeleteTaskUseCase<'a, R, C> {
    tasks: &'a R,
    clock: &'a C,
}

impl<'a, R, C> DeleteTaskUseCase<'a, R, C>
where
    R: TaskRepository,
    C: Clock,
{
    pub const fn new(tasks: &'a R, clock: &'a C) -> Self {
        Self { tasks, clock }
    }

    pub async fn execute(&self, task_id: TaskId) -> ApplicationResult<Task> {
        let mut task =
            self.tasks
                .find_task(task_id)
                .await?
                .ok_or_else(|| ApplicationError::NotFound {
                    entity: "task",
                    id: task_id.to_string(),
                })?;
        task.delete(self.clock.now())?;
        self.tasks.save_task(&task).await?;
        Ok(task)
    }
}

pub fn active_tasks(tasks: Vec<Task>) -> Vec<Task> {
    tasks
        .into_iter()
        .filter(|task| task.status == TaskStatus::Active)
        .collect()
}
