use std::error::Error as StdError;
use std::env;
use std::process;
use std::ffi::OsString;
use std::time::Duration;

use rusqlite::Connection;
use rusqlite::params;
use clap::{Parser, Subcommand};

mod commands;
mod types;
mod errors;

use types::*;
use commands::*;
use errors::{AppResult, TaskError, CommandError};

fn main() {
    match app_main() {
        Ok(()) => {},
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

/// All possible commands. Used for prefix matching.
static COMMANDS: &[&str] = &[
    "push",
    "backpush",
    "pop",
    "kill",
    "ls",
    "swap",
    "clear",
    "clearall",
    "newstack",
    "switchto",
    "dropstack",
    "liststacks",
    "triggerreminder",
    "remindme"
];

#[derive(Parser)]
#[command(version = "0.3.2", about = "Stack-based task tracker", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command
}

#[derive(Subcommand)]
enum Command {
    /// Push a task onto the stack
    Push {
        /// Task text to use
        task: String,
    },
    /// Push a task onto the bottom of the stack.
    Backpush {
        /// Task description
        task: String,
    },
    /// Pop a task from the top of the stack
    Pop {
        /// Name of the stack to push onto
        name: Option<String>,
    },
    /// List all tasks on the current stack.
    Ls,
    /// Swap two tasks
    Swap {
        task1: TaskIndex,
        task2: TaskIndex,
    },
    /// Clear all tasks on the current stack.
    Clear,
    /// Wipe all stacks clean.
    Clearall,
    /// Create a new stack
    Newstack {
        /// Name of the new stack. Must not be the same as an existing stack's name!
        name: String,
    },
    /// Delete a task.
    Kill {
        task: TaskIndex,
    },
    /// Switch to another stack.
    Switchto {
        /// Stack to switch to. Must exist.
        stack: String,
    },
    /// Delete a stack and all its items.
    Dropstack {
        stack: String,
    },
    /// List all stacks.
    Liststacks,
    /// Trigger a previously-created reminder.
    Triggerreminder {
        reminder_id: String,
    },
    /// Create a task reminder at some future point in time.
    Remindme {
        /// Task to remind me of. If the task is completed, the reminder will not trigger.
        task: TaskIndex,
        /// How long to wait. Specified as ([1-9][0-9]*h)?([1-9][0-9]*m)?([1-9][0-9]*s)?
        delay: String,
    }
}



fn app_main() -> Result<(), Box<dyn StdError>> {
    let mut os_args: Vec<OsString> = env::args_os().collect();
    if os_args.len() > 1 {
        let raw_command = os_args[1].to_str().unwrap();
        os_args[1] = resolve_command(raw_command)?.into();

    }
    let os_args = os_args;
    let cli = Cli::parse_from(os_args.into_iter());
    let mut db_path = std::env::temp_dir();
    db_path.push("yakstack.db");
    let mut conn = Connection::open(&db_path)
                              .map_err(|e| format!("unable to open yakstack database: {}", e))?;
    // DB could be locked by a previous remind command.
    conn.busy_timeout(Duration::from_secs(1))?;
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    if !is_db_initialized(&conn) {
        init_db(&mut conn)?;
    }
    match cli.command {
        Command::Push { task }=> {
            push_task(&conn, task)?;
        },
        Command::Backpush { task }=> {
            pushback_task(&conn, task)?;
        },
        Command::Pop { name }=> {
            if let Some(name) = name {
                return Ok(pop_to(&conn, name)?);
            }

            if let Some(task) = pop_task(&conn)? {
                println!("{} ✔️", task);
            } else {
                return Err(TaskError::NoTasks.into());
            }
        }
        Command::Swap { task1, task2 }=> {
            swap_tasks(&mut conn, task1, task2)?;
        }
        Command::Clear => clear_tasks(&conn)?,
        Command::Clearall => clear_all_tasks(&conn)?,
        Command::Ls => {
            println!("Stack: {}", get_current_stack_name(&conn)?);
            list_tasks(&conn)?.iter().enumerate().for_each(|(i, task)| println!("{}. {}", i, task));
        }
        Command::Newstack { name } => new_stack(&conn, name)?,
        Command::Switchto { stack } => switch_to_stack(&conn, stack)?,
        Command::Dropstack { stack } => drop_stack(&mut conn, stack)?,
        Command::Liststacks => list_stacks(&conn)?.iter().for_each(|stack| println!("{}", stack)),
        Command::Kill { task }=> {
            let killed = kill_task(&mut conn, task)?;
            println!("{} 🗑️", killed);
        }
        Command::Remindme { task, delay }=> remind_me(&mut conn, task, delay)?,
        Command::Triggerreminder { reminder_id }=> trigger_reminder(db_path, conn, reminder_id)?,
    }
    Ok(())
}

/// Resolve a `prefix` into its full command.
fn resolve_command(prefix: &str) -> Result<&str, CommandError>  {
    if prefix.starts_with('-') {
        return Ok(prefix);
    }
    let mut matcher: &str = "";
    let mut num_matches = 0;
    for &c in COMMANDS {
        // Commands may be prefixes of others.
        if c == prefix {
            matcher = c;
            num_matches = 1;
            break;
        } else if c.starts_with(prefix) {
            matcher = c;
            num_matches += 1;
        }
    }

    if num_matches == 0 {
        Err(CommandError::NoMatchingCommand(prefix.into()))
    } else if num_matches > 1 {
        Err(CommandError::AmbiguousPrefix(prefix.into()))
    } else {
        Ok(matcher)
    }
}

mod tests {
    use crate::resolve_command;
    use crate::errors::CommandError;

    #[test]
    fn resolve_command_test() {
        assert!(matches!(resolve_command("l"), Err(CommandError::AmbiguousPrefix(_))));
        assert!(matches!(resolve_command("xxx"), Err(CommandError::NoMatchingCommand(_))));
        assert!(matches!(resolve_command("b"), Ok("backpush")));
    }

    #[test]
    fn resolve_command_command_prefixes_other_command_works() {
        assert!(matches!(resolve_command("clear"), Ok("clear")));
    }
}

/// Check whether `db` is initialized.
fn is_db_initialized(db: &Connection) -> bool {
    get_current_stack_id(db).is_ok()
}

/// Initialize `db` with application tables.
fn init_db(db: &mut Connection) -> AppResult<()> {
    let xact = db.transaction()?;
    xact.execute("PRAGMA foreign_keys = ON", [])?;
    xact.execute("CREATE TABLE IF NOT EXISTS stacks(id INTEGER PRIMARY KEY, name TEXT NOT NULL, UNIQUE(name))", [])?;
    xact.execute("CREATE TABLE IF NOT EXISTS app_state(stack_id INTEGER NOT NULL, FOREIGN KEY(stack_id) REFERENCES stacks(id))", [])?;
    xact.execute("CREATE TABLE IF NOT EXISTS tasks(task TEXT NOT NULL, task_order INTEGER NOT NULL, id INTEGER PRIMARY KEY, stack_id INTEGER NOT NULL, FOREIGN KEY(stack_id) REFERENCES stacks(id), CHECK (task_order = task_order))", [])?;
    // reminders PK should be a UUID
    xact.execute("CREATE TABLE IF NOT EXISTS reminders(id TEXT PRIMARY KEY, delay INTEGER NOT NULL, task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, CHECK (delay > 0))", [])?;
    xact.execute("CREATE INDEX IF NOT EXISTS tasks_ix ON tasks(stack_id, task_order, task)", [])?;
    xact.execute("INSERT INTO stacks(id, name) VALUES (?, 'default')", params![DEFAULT_STACK_ID])?;
    xact.execute("INSERT INTO app_state(stack_id) VALUES (?)", params![DEFAULT_STACK_ID])?;
    xact.commit()?;
    Ok(())
}

