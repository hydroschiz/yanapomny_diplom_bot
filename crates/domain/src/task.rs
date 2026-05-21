use chrono::{DateTime, Utc};

use crate::{DomainError, TaskId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Active,
    Completed,
    Deleted,
}

impl TaskStatus {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: Option<TaskId>,
    pub user_id: UserId,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub due_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(user_id: UserId, title: impl Into<String>, created_at: DateTime<Utc>) -> Self {
        Self {
            id: None,
            user_id,
            title: title.into(),
            description: None,
            status: TaskStatus::Active,
            priority: TaskPriority::Normal,
            due_at: None,
            created_at,
            updated_at: created_at,
        }
    }

    pub fn assign_id(&mut self, id: TaskId) {
        self.id = Some(id);
    }

    pub fn set_description(&mut self, description: impl Into<String>, now: DateTime<Utc>) {
        self.description = Some(description.into());
        self.updated_at = now;
    }

    pub fn set_due_at(&mut self, due_at: Option<DateTime<Utc>>, now: DateTime<Utc>) {
        self.due_at = due_at;
        self.updated_at = now;
    }

    pub fn set_priority(&mut self, priority: TaskPriority, now: DateTime<Utc>) {
        self.priority = priority;
        self.updated_at = now;
    }

    pub fn complete(&mut self, now: DateTime<Utc>) -> Result<(), DomainError> {
        if self.status != TaskStatus::Active {
            return Err(DomainError::InvalidStatusTransition {
                from: self.status.name(),
                to: "completed",
            });
        }
        self.status = TaskStatus::Completed;
        self.updated_at = now;
        Ok(())
    }

    pub fn delete(&mut self, now: DateTime<Utc>) -> Result<(), DomainError> {
        if self.status == TaskStatus::Deleted {
            return Err(DomainError::InvalidStatusTransition {
                from: self.status.name(),
                to: "deleted",
            });
        }
        self.status = TaskStatus::Deleted;
        self.updated_at = now;
        Ok(())
    }
}
