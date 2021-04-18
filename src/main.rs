use std::error::Error;
use std::mem;

use rusqlite::{Connection, OptionalExtension};
use rusqlite::params;
use rusqlite::Result as RusqliteResult;
use clap::{Arg, App, SubCommand, AppSettings};

fn main() -> Result<(), Box<dyn Error>> {
    let matches = App::new("yakstack")
                      .version("0.1")
                      .about("yak-shaving stack")
                      .setting(AppSettings::SubcommandRequiredElseHelp)
                      .subcommand(SubCommand::with_name("push")
                          .about("Push a task onto the stack")
                          .arg(Arg::with_name("TASK")
                                .help("task description")
                                .required(true)
                                .takes_value(true)))
                      .subcommand(SubCommand::with_name("pop")
                          .about("Pop a task from the top of the stack"))
                      .subcommand(SubCommand::with_name("ls")
                          .about("List all tasks"))
                      .subcommand(SubCommand::with_name("clear")
                          .about("Clear all tasks"))
                      .get_matches();
    let mut db_path = std::env::temp_dir();
    db_path.push("yakstack.db");
    let conn = Connection::open(db_path)
                          .map_err(|e| format!("unable to open yakstack database: {}", e))?;
    conn.execute("CREATE TABLE IF NOT EXISTS tasks(task TEXT NOT NULL, task_order INTEGER PRIMARY KEY AUTOINCREMENT)", [])?;
    match matches.subcommand() {
        ("push", submatches) => {
            let task = submatches.unwrap().value_of("TASK").unwrap();
            push_task(&conn, task.into())?;
        },
        ("pop", _) => {
            if let Some(task) = pop_task(&conn)? {
                println!("{} ✔️", task);
            } else {
                return Err("No tasks to pop!".into());
            }
        }
        ("clear", _) => clear_tasks(&conn)?,
        ("ls", _) => {
            let tasks = list_tasks(&conn)?;
            tasks.iter().enumerate().for_each(|(i, task)| println!("{}. {}", i, task));
            // let OS clean up after us
            mem::forget(tasks);
        }
        _ => unreachable!("No subcommand provided")
    }
    Ok(())
}

fn push_task(db: &Connection, task: String) -> RusqliteResult<usize> {
    // https://sqlite.org/autoinc.html
    db.execute("INSERT INTO tasks(task) VALUES (?)", params![task])
}

fn pop_task(db: &Connection) -> RusqliteResult<Option<String>> {
    let maybe_task: Option<String> = db.query_row("SELECT task FROM tasks WHERE task_order = (SELECT max(task_order) FROM tasks)", [], |row| row.get(0)).optional()?;
    db.execute("DELETE FROM tasks WHERE task_order = (SELECT max(task_order) FROM tasks)", [])?;
    Ok(maybe_task)
}

fn clear_tasks(db: &Connection) -> RusqliteResult<()> {
    db.execute("DELETE FROM tasks WHERE 1 = 1", [])?;
    Ok(())
}

fn list_tasks(db: &Connection) -> RusqliteResult<Vec<String>> {
    let mut stmt = db.prepare("SELECT task FROM tasks ORDER BY task_order")?;
    let result = stmt.query_map([], |row| row.get(0))?.collect();
    result
}
