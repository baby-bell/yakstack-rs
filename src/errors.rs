
use rusqlite::Error as RusqliteError;
use thiserror::Error;

use crate::types::*;

/// Errors related to stack management.
#[derive(Error, Debug)]
pub enum StackError {
    #[error("no such stack: '{0}'")]
    NoSuchStack(String),
    #[error("stack '{0}' already exists")]
    StackAlreadyExists(String),
    #[error("can't delete default stack")]
    CantDeleteDefaultStack,
    #[error("can't delete current stack")]
    CantDeleteCurrentStack
}

/// Errors related to task management.
#[derive(Error, Debug)]
pub enum TaskError {
    #[error("no tasks!")]
    NoTasks,
    #[error("task #{0} doesn't exist")]
    NoSuchTask(TaskIndex),
    #[error("tasks #{0} and #{1} don't exist")]
    NoSuchTasks(TaskIndex, TaskIndex)
}

#[derive(Error, Debug)]
pub enum CommandError {
    #[error("could not find command matching '{0}'")]
    NoMatchingCommand(String),
    #[error("more than one command matches '{0}'")]
    AmbiguousPrefix(String)
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("{0}")]
    Stack(#[from] StackError),
    #[error("{0}")]
    Task(#[from] TaskError),
    #[error("database error: {0}")]
    Sqlite(#[from] RusqliteError),
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Environment(String),
}

pub type AppResult<T> = Result<T, AppError>;
