use std::error::Error as StdError;
use std::env;
use std::process;
use std::ffi::OsString;

use rusqlite::Connection;
use rusqlite::params;
use clap::{Arg, App, SubCommand, AppSettings};

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
    "ls",
    "swap",
    "clear",
    "clearall",
    "newstack",
    "switchto",
    "dropstack",
    "liststacks"
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
        .settings(&[AppSettings::SubcommandRequiredElseHelp])
        .subcommand(SubCommand::with_name("push")
            .about("Push a task onto the stack")
            .arg(Arg::with_name("TASK")
                    .help("task description")
                    .required(true)
                    .takes_value(true)))
        .subcommand(SubCommand::with_name("backpush")
            .about("Push a task onto the bottom of the stack")
            .arg(Arg::with_name("TASK")
                .help("task description")
                .required(true)
                .takes_value(true)))
        .subcommand(SubCommand::with_name("pop")
            .about("Pop a task from the top of the stack")
            .arg(Arg::with_name("NAME")
                .help("name of the stack to push onto")
                .required(false)
                .takes_value(true)))
        .subcommand(SubCommand::with_name("ls")
            .about("List all tasks"))
        .subcommand(SubCommand::with_name("swap")
            .about("Swap two tasks")
            .arg(Arg::with_name("TASK1")
                .help("first task")
                .required(true)
                .takes_value(true)
                .validator(is_task_index))
            .arg(Arg::with_name("TASK2")
                .help("second task")
                .required(true)
                .takes_value(true)
                .validator(is_task_index)))
        .subcommand(SubCommand::with_name("clear")
            .about("Clear all tasks on the current stack"))
        .subcommand(SubCommand::with_name("clearall")
            .about("Clear all tasks from all stacks"))   
        .subcommand(SubCommand::with_name("newstack")
            .about("Create a new stack")
            .arg(Arg::with_name("NAME")
                .help("name of the stack")
                .required(true)
                .takes_value(true)))
        .subcommand(SubCommand::with_name("switchto")
            .about("Switch to another stack")
            .arg(Arg::with_name("NAME")
                .help("name of the stack to switch to")
                .required(true)
                .takes_value(true)))
        .subcommand(SubCommand::with_name("dropstack")
            .about("Drop a stack")
            .arg(Arg::with_name("NAME")
                .help("name of the stack to drop. Must not be default or current stack")
                .required(true)
                .takes_value(true)))
        .subcommand(SubCommand::with_name("liststacks")
            .about("List all stacks"))
        .get_matches_from(&os_args);
    let mut db_path = std::env::temp_dir();
    db_path.push("yakstack.db");
    let mut conn = Connection::open(db_path)
                              .map_err(|e| format!("unable to open yakstack database: {}", e))?;
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    if !is_db_initialized(&conn) {
        init_db(&mut conn)?;
    }
    match matches.subcommand() {
        ("push", submatches) => {
            let task = submatches.unwrap().value_of("TASK").unwrap();
            push_task(&conn, task.into())?;
        },
        ("backpush", submatches) => {
            let task = submatches.unwrap().value_of("TASK").unwrap();
            pushback_task(&conn, task.into())?;
        },
        ("pop", submatches) => {
            if let Some(destination_stack) = submatches.unwrap().value_of("NAME") {
                return Ok(pop_to(&conn, destination_stack.into())?);
            }

            if let Some(task) = pop_task(&conn)? {
                println!("{} ✔️", task);
            } else {
                return Err(TaskError::NoTasks.into());
            }
        }
        ("swap", submatches) => {
            let submatches = submatches.unwrap();
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
            let name = submatches.unwrap().value_of("NAME").unwrap();
            new_stack(&conn, name.into())?;
        }
        ("switchto", submatches) => {
            let name = submatches.unwrap().value_of("NAME").unwrap();
            switch_to_stack(&conn, name.into())?;
        }
        ("dropstack", submatches) => {
            let name = submatches.unwrap().value_of("NAME").unwrap();
            drop_stack(&mut conn, name.into())?;
        }
        ("liststacks", _) => {
            list_stacks(&conn)?.iter().for_each(|stack| println!("{}", stack));
        }
        _ => unreachable!("No subcommand provided")
    }
    Ok(())
}

/// Resolve a `prefix` into its full command.
fn resolve_command(prefix: &str) -> Result<&str, CommandError>  {
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

fn is_task_index(arg: String) -> Result<(), String> {
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
    xact.execute("CREATE INDEX IF NOT EXISTS tasks_ix ON tasks(stack_id, task_order, task)", [])?;
    xact.execute("INSERT INTO stacks(id, name) VALUES (?, 'default')", params![DEFAULT_STACK_ID])?;
    xact.execute("INSERT INTO app_state(stack_id) VALUES (?)", params![DEFAULT_STACK_ID])?;
    xact.commit()?;
    Ok(())
}

