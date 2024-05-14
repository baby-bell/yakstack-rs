use std::error::Error as StdError;
use std::env;
use std::process;
use std::ffi::OsString;
use std::time::Duration;

use rusqlite::Connection;
use rusqlite::params;
use clap::{Arg, App, Command};
use uuid::Uuid;

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

fn app_main() -> Result<(), Box<dyn StdError>> {
    let mut os_args: Vec<OsString> = env::args_os().collect();
    if os_args.len() > 1 {
        let raw_command = os_args[1].to_str().unwrap();
        os_args[1] = resolve_command(raw_command)?.into();

    }
    let os_args = os_args;
    let matches = App::new("yakstack")
        .version("0.3")
        .about("yak-shaving stack")
        .subcommand_required(true)
        .subcommand(Command::new("push")
            .about("Push a task onto the stack")
            .arg(Arg::new("TASK")
                    .help("task description")
                    .required(true)
                    .takes_value(true)))
        .subcommand(Command::new("backpush")
            .about("Push a task onto the bottom of the stack")
            .arg(Arg::new("TASK")
                .help("task description")
                .required(true)
                .takes_value(true)))
        .subcommand(Command::new("pop")
            .about("Pop a task from the top of the stack")
            .arg(Arg::new("NAME")
                .help("name of the stack to push onto")
                .required(false)
                .takes_value(true)))
        .subcommand(Command::new("ls")
            .about("List all tasks"))
        .subcommand(Command::new("swap")
            .about("Swap two tasks")
            .arg(Arg::new("TASK1")
                .help("first task")
                .required(true)
                .takes_value(true)
                .validator(is_task_index))
            .arg(Arg::new("TASK2")
                .help("second task")
                .required(true)
                .takes_value(true)
                .validator(is_task_index)))
        .subcommand(Command::new("clear")
            .about("Clear all tasks on the current stack"))
        .subcommand(Command::new("clearall")
            .about("Clear all tasks from all stacks"))   
        .subcommand(Command::new("newstack")
            .about("Create a new stack")
            .arg(Arg::new("NAME")
                .help("name of the stack")
                .required(true)
                .takes_value(true)))
        .subcommand(Command::new("kill")
            .about("Delete a task")
            .arg(Arg::new("TASK")
                .help("task to delete")
                .required(true)
                .takes_value(true)
                .validator(is_task_index)))
        .subcommand(Command::new("switchto")
            .about("Switch to another stack")
            .arg(Arg::new("NAME")
                .help("name of the stack to switch to")
                .required(true)
                .takes_value(true)))
        .subcommand(Command::new("dropstack")
            .about("Delete a stack and all its items")
            .arg(Arg::new("NAME")
                .help("name of the stack to drop. Must not be default or current stack")
                .required(true)
                .takes_value(true)))
        .subcommand(Command::new("liststacks")
            .about("List all stacks"))
        .subcommand(Command::new("triggerreminder")
            .about("Trigger a reminder as specified in the reminder table")
            .arg(Arg::new("REMINDER_ID")
                .required(true)
                .takes_value(true)))
        .subcommand(Command::new("remindme")
            .about("Remind me about a task at some future point in time")
            .arg(Arg::new("TASK")
                .help("task to remind about")
                .required(true)
                .takes_value(true))
            .arg(Arg::new("DELAY")
                .help("time to wait before reminding")
                .required(true)
                .takes_value(true)))
        .get_matches_from(&os_args);
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
    match matches.subcommand().expect("No subcommand provided, bug") {
        ("push", submatches) => {
            let task = submatches.value_of("TASK").unwrap();
            push_task(&conn, task.into())?;
        },
        ("backpush", submatches) => {
            let task = submatches.value_of("TASK").unwrap();
            pushback_task(&conn, task.into())?;
        },
        ("pop", submatches) => {
            if let Some(destination_stack) = submatches.value_of("NAME") {
                return Ok(pop_to(&conn, destination_stack.into())?);
            }

            if let Some(task) = pop_task(&conn)? {
                println!("{} ✔️", task);
            } else {
                return Err(TaskError::NoTasks.into());
            }
        }
        ("swap", submatches) => {
            let task1: TaskIndex = submatches.value_of("TASK1").unwrap().parse().unwrap();
            let task2: TaskIndex = submatches.value_of("TASK2").unwrap().parse().unwrap();
            swap_tasks(&mut conn, task1, task2)?;
        }
        ("clear", _) => clear_tasks(&conn)?,
        ("clearall", _) => clear_all_tasks(&conn)?,
        ("ls", _) => {
            println!("Stack: {}", get_current_stack_name(&conn)?);
            list_tasks(&conn)?.iter().enumerate().for_each(|(i, task)| println!("{}. {}", i, task));
        }
        ("newstack", submatches) => {
            let name = submatches.value_of("NAME").unwrap();
            new_stack(&conn, name.into())?;
        }
        ("switchto", submatches) => {
            let name = submatches.value_of("NAME").unwrap();
            switch_to_stack(&conn, name.into())?;
        }
        ("dropstack", submatches) => {
            let name = submatches.value_of("NAME").unwrap();
            drop_stack(&mut conn, name.into())?;
        }
        ("liststacks", _) => {
            list_stacks(&conn)?.iter().for_each(|stack| println!("{}", stack));
        }
        ("kill", submatches) => {
            let task: TaskIndex = submatches.value_of("TASK").unwrap().parse().unwrap();
            let killed = kill_task(&mut conn, task)?;
            println!("{} 🗑️", killed);
        }
        ("remindme", submatches) => {
            let task: TaskIndex = submatches.value_of("TASK").unwrap().parse().unwrap();
            let time_spec = submatches.value_of("DELAY").unwrap();
            remind_me(&mut conn, task, time_spec.into())?;
        }
        ("triggerreminder", submatches) => {
            let reminder_id: String = submatches.value_of("REMINDER_ID")
                .expect("missing REMINDER_ID")
                .into();
            trigger_reminder(db_path, conn, reminder_id)?;
        }
        _ => unreachable!("No subcommand provided")
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

fn is_task_index<'a>(arg: &'a str) -> Result<(), String> {
    let _: TaskIndex = arg.parse().map_err(|e| format!("{} is not a valid unsigned number: {}", arg, e))?;
    Ok(())
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

