use thiserror::Error;

pub type ApplicationResult<T> = Result<T, ApplicationError>;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Domain(#[from] domain::DomainError),

    #[error("{entity} `{id}` was not found")]
    NotFound { entity: &'static str, id: String },

    #[error("repository error: {0}")]
    Repository(String),

    #[error("external service error: {0}")]
    ExternalService(String),

    #[error("conflict: {0}")]
    Conflict(String),
}
