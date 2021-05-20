use std::error::Error;
use std::cmp;

use rusqlite::{Connection, OptionalExtension};
use rusqlite::params;
use rusqlite::Result as RusqliteResult;
use clap::{Arg, App, SubCommand, AppSettings};

type StackId = u32;
const DEFAULT_STACK_ID: StackId = 1;

fn main() -> Result<(), Box<dyn Error>> {
    let matches = App::new("yakstack")
        .version("0.2")
        .about("yak-shaving stack")
        .settings(&[AppSettings::SubcommandRequiredElseHelp])
        .subcommand(SubCommand::with_name("push")
            .about("Push a task onto the stack")
            .arg(Arg::with_name("TASK")
                    .help("task description")
                    .required(true)
                    .takes_value(true)))
        .subcommand(SubCommand::with_name("pushback")
            .about("Push a task onto the bottom of the stack")
            .arg(Arg::with_name("TASK")
                .help("task description")
                .required(true)
                .takes_value(true)))
        .subcommand(SubCommand::with_name("pop")
            .about("Pop a task from the top of the stack"))
        .subcommand(SubCommand::with_name("ls")
            .about("List all tasks"))
        .subcommand(SubCommand::with_name("clear")
            .about("Clear all tasks on the current stack"))
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
        .get_matches();
    let mut db_path = std::env::temp_dir();
    db_path.push("yakstack.db");
    let mut conn = Connection::open(db_path)
                              .map_err(|e| format!("unable to open yakstack database: {}", e))?;
    if !is_db_initialized(&conn) {
        init_db(&mut conn)?;
    }
    match matches.subcommand() {
        ("push", submatches) => {
            let task = submatches.unwrap().value_of("TASK").unwrap();
            push_task(&conn, task.into())?;
        },
        ("pushback", submatches) => {
            let task = submatches.unwrap().value_of("TASK").unwrap();
            pushback_task(&conn, task.into())?;
        },
        ("pop", _) => {
            if let Some(task) = pop_task(&conn)? {
                println!("{} ✔️", task);
            } else {
                println!("No tasks!");
                std::process::exit(1);
            }
        }
        ("clear", _) => clear_tasks(&conn)?,
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

fn is_db_initialized(db: &Connection) -> bool {
    get_current_stack_id(db).is_ok()
}

fn init_db(db: &mut Connection) -> RusqliteResult<()> {
    let tx = db.transaction()?;
    tx.execute("PRAGMA foreign_keys = ON", [])?;
    tx.execute("CREATE TABLE IF NOT EXISTS stacks(id INTEGER PRIMARY KEY, name TEXT NOT NULL, UNIQUE(name))", [])?;
    println!("Creating app_state");
    tx.execute("CREATE TABLE IF NOT EXISTS app_state(stack_id INTEGER NOT NULL, FOREIGN KEY(stack_id) REFERENCES stacks(id))", [])?;
    println!("Creating tasks");
    tx.execute("CREATE TABLE IF NOT EXISTS tasks(task TEXT NOT NULL, task_order REAL NOT NULL, id INTEGER PRIMARY KEY, stack_id INTEGER NOT NULL, FOREIGN KEY(stack_id) REFERENCES stacks(id), CHECK (task_order = task_order))", [])?;
    println!("Creating index");
    tx.execute("CREATE INDEX IF NOT EXISTS tasks_stacks_fk_ix ON tasks(stack_id)", [])?;
    tx.execute("INSERT INTO stacks(id, name) VALUES (?, 'default')", params![DEFAULT_STACK_ID])?;
    println!("Inserting into app_state");
    tx.execute("INSERT INTO app_state(stack_id) VALUES (?)", params![DEFAULT_STACK_ID])?;
    tx.commit()?;
    Ok(())
}

fn get_current_stack_id(db: &Connection) -> RusqliteResult<StackId> {
    let stack_id: StackId = db.query_row("SELECT stack_id FROM app_state", [], |row| row.get(0))?;
    Ok(stack_id)
}

fn get_current_stack_name(db: &Connection) -> RusqliteResult<String> {
    let current_stack_id = get_current_stack_id(db)?;
    let current_stack_name: String = db.query_row("SELECT name FROM stacks WHERE id = ?", params![current_stack_id], |row| row.get(0))?;
    Ok(current_stack_name)
}


fn push_task(db: &Connection, task: String) -> RusqliteResult<usize> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (?, (SELECT max(task_order) + 1 OR 1.0 FROM tasks), ?)", params![task, current_stack_id])
}

fn pushback_task(db: &Connection, task: String) -> RusqliteResult<usize> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (?, (SELECT min(task_order) - 1 OR 1.0 FROM tasks), ?)", params![task, current_stack_id])
}

fn pop_task(db: &Connection) -> RusqliteResult<Option<String>> {
    let current_stack_id = get_current_stack_id(db)?;
    let maybe_task_id: Option<i64> = db.query_row("SELECT id
    FROM tasks
    WHERE task_order = (SELECT max(task_order) FROM tasks WHERE stack_id = ?)
    AND stack_id = ?", params![current_stack_id, current_stack_id], |row| row.get(0)).optional()?;

    if let Some(task_id) = maybe_task_id {
        let task: String = db.query_row("SELECT task FROM tasks WHERE id = ?", params![task_id], |row| row.get(0))?;
        db.execute("DELETE FROM tasks WHERE id = ?", params![task_id])?;
        Ok(Some(task))
    } else {
        Ok(None)
    }
}

fn clear_tasks(db: &Connection) -> RusqliteResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("DELETE FROM tasks WHERE stack_id = ?", params![current_stack_id])?;
    Ok(())
}

fn new_stack(db: &Connection, stack_name: String) -> Result<(), Box<dyn Error>> {
    let stack_exists: Option<i32> = db.query_row("SELECT 1 FROM stacks WHERE name = ?", params![stack_name], |row| row.get(0)).optional()?;
    if let Some(_) = stack_exists {
        return Err(format!("Stack {} already exists!", stack_name).into());
    }

    db.execute("INSERT INTO stacks(name) VALUES (?)", params![stack_name])?;
    Ok(())
}

fn stack_name_to_id(db: &Connection, name: &str) -> RusqliteResult<Option<StackId>> {
    let maybe_stack_id: Option<StackId> = db.query_row("SELECT id FROM stacks WHERE name = ?",
        params![name], |row| row.get(0)).optional()?;
    Ok(maybe_stack_id)
}

fn drop_stack(db: &mut Connection, stack_name: String) -> Result<(), Box<dyn Error>> {
    let current_stack_id = get_current_stack_id(db)?;
    let stack_id = stack_name_to_id(db, &stack_name)?;
    match stack_id {
        None => return Err(format!("Stack {} doesn't exist!", stack_name).into()),
        Some(id) => {
            if id == DEFAULT_STACK_ID {
                return Err("Can't delete the default stack".into());
            } else if id == current_stack_id {
                return Err("Can't delete the current stack".into());
            }
            let tx = db.transaction()?;
            tx.execute("DELETE FROM tasks WHERE stack_id = ?", params![id])?;
            tx.execute("DELETE FROM stacks WHERE id = ?", params![id])?;
            tx.commit()?;
        }
    }
    Ok(())
}

fn switch_to_stack(db: &Connection, stack_name: String) -> Result<(), Box<dyn Error>> {
    match stack_name_to_id(db, &stack_name)? {
        None => return Err(format!("Stack {} doesn't exist", stack_name).into()),
        Some(id) => {
            db.execute("UPDATE app_state SET stack_id = ?", params![id])?;
            Ok(())
        }
    }
}

fn list_stacks(db: &Connection) -> RusqliteResult<Vec<String>> {
    let mut stmt = db.prepare("SELECT * FROM stacks")?;
    let result = stmt.query_map([], |row| row.get(0))?.collect();
    result
}


fn list_tasks(db: &Connection) -> RusqliteResult<Vec<String>> {
    let current_stack_id = get_current_stack_id(db)?;
    let mut stmt = db.prepare("SELECT task FROM tasks WHERE stack_id = ? ORDER BY task_order")?;
    let result = stmt.query_map(params![current_stack_id], |row| row.get(0))?.collect();
    result
}

// TODO: this
fn swap_tasks(db: &Connection, idx1: u64, idx2: u64) -> Result<(), Box<dyn Error>> {
    let task_count: u64 = db.query_row("SELECT count(*) FROM tasks", [], |row| row.get(0))?;
    match (idx1 >= task_count, idx2 >= task_count) {
        (false, false) => {}
        (true, false) | (false, true) => {
            let invalid = cmp::max(idx1, idx2);
            return Err(format!("{} is an invalid task index", invalid).into());
        }
        (true, true) => {
            return Err(format!("{} and {} are invalid task indices", idx1, idx2).into());
        }
    }


    Ok(())
}
