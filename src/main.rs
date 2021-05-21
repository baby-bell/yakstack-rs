use std::error::Error;
use std::cmp;

use rusqlite::Error as RusqliteError;
use rusqlite::{Connection, OptionalExtension, named_params};
use rusqlite::params;
use rusqlite::Result as RusqliteResult;
use clap::{Arg, App, SubCommand, AppSettings};

enum StackError {
    NoSuchStack(String),
    StackAlreadyExists(String),
    CantDeleteDefaultStack,
    CantDeleteCurrentStack
}

enum TaskError {
    NoTasks,
    NoSuchTask(TaskIndex),
    NoSuchTasks(TaskIndex)
}

enum AppError {
    Stack(StackError),
    Task(TaskError),
    Sqlite(RusqliteError)
}

type StackId = u32;
type TaskIndex = u32;
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
                .validator(is_u64))
            .arg(Arg::with_name("TASK2")
                .help("second task")
                .required(true)
                .takes_value(true)
                .validator(is_u64)))
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
        .get_matches();
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
                return pop_to(&conn, destination_stack.into());
            }

            if let Some(task) = pop_task(&conn)? {
                println!("{} ✔️", task);
            } else {
                return Err("No tasks!".into());
            }
        }
        ("swap", submatches) => {
            let submatches = submatches.unwrap();
            let task1: u64 = submatches.value_of("TASK1").unwrap().parse().unwrap();
            let task2: u64 = submatches.value_of("TASK2").unwrap().parse().unwrap();
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

fn is_u64(arg: String) -> Result<(), String> {
    let _: u64 = arg.parse().map_err(|e| format!("{} is not a valid unsigned number: {}", arg, e))?;
    Ok(())
}

fn is_db_initialized(db: &Connection) -> bool {
    get_current_stack_id(db).is_ok()
}

fn init_db(db: &mut Connection) -> RusqliteResult<()> {
    let xact = db.transaction()?;
    xact.execute("PRAGMA foreign_keys = ON", [])?;
    xact.execute("CREATE TABLE IF NOT EXISTS stacks(id INTEGER PRIMARY KEY, name TEXT NOT NULL, UNIQUE(name))", [])?;
    xact.execute("CREATE TABLE IF NOT EXISTS app_state(stack_id INTEGER NOT NULL, FOREIGN KEY(stack_id) REFERENCES stacks(id))", [])?;
    xact.execute("CREATE TABLE IF NOT EXISTS tasks(task TEXT NOT NULL, task_order INTEGER NOT NULL, id INTEGER PRIMARY KEY, stack_id INTEGER NOT NULL, FOREIGN KEY(stack_id) REFERENCES stacks(id), CHECK (task_order = task_order))", [])?;
    xact.execute("CREATE INDEX IF NOT EXISTS task_order_ix ON tasks(task_order, task)", [])?;
    xact.execute("CREATE INDEX IF NOT EXISTS tasks_stacks_fk_ix ON tasks(stack_id)", [])?;
    xact.execute("INSERT INTO stacks(id, name) VALUES (?, 'default')", params![DEFAULT_STACK_ID])?;
    xact.execute("INSERT INTO app_state(stack_id) VALUES (?)", params![DEFAULT_STACK_ID])?;
    xact.commit()?;
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
    db.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (?, (SELECT coalesce(max(task_order) + 1, 1) FROM tasks), ?)", params![task, current_stack_id])
}

fn pushback_task(db: &Connection, task: String) -> RusqliteResult<usize> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (?, (SELECT coalesce(min(task_order) - 1, 1) FROM tasks), ?)", params![task, current_stack_id])
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

fn clear_all_tasks(db: &Connection) -> RusqliteResult<()> {
    db.execute("DELETE FROM tasks WHERE 1 = 1", [])?;
    Ok(())
}

/// Insert `task` after the `task_index`th task, starting from 0.
/// 
/// i.e. if `task_index == 0`, then this is equivalent to `backpush`
fn insert_after(db: &mut Connection, task_index: u32, task: String) -> Result<(), Box<dyn Error>> {
    // two cases: task is last and task is not last
    // if task is not last, avg() works
    // if task is last, avg() just gives task order
    let current_stack_id = get_current_stack_id(db)?;
    let num_tasks = db.query_row("SELECT count(*) FROM tasks WHERE stack_id = ?", params![current_stack_id], |row| row.get(0))?;
    if task_index >= num_tasks {
        return Err(format!("task index {} is out of bounds", task_index).into())
    } else if task_index == 0 {
        return push_task(db, task)
            .map(|_| ())
            .map_err(Into::into);
    } else if task_index == num_tasks - 1 {
        return pushback_task(db, task)
            .map(|_| ())
            .map_err(Into::into);

    }

    // sqlite starts rows from 1
    let task_index = task_index + 1;
    // task is not last, we are good to go.
    let task_order: u32 = db.query_row("SELECT task_order + 1 FROM (SELECT row_number() OVER (ORDER BY task_order) task_index, task_order FROM tasks) WHERE task_index = :task_index", 
    named_params! {":task_index": task_index}, |row| row.get(0))?;
    let xact = db.transaction()?;
    xact.execute("UPDATE tasks SET task_order = task_order + 1 WHERE task_order >= :task_order AND stack_id = :stack_id", named_params! {":task_order": task_order, ":stack_id": current_stack_id})?;
    xact.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (:task, :task_order, :stack_id)", named_params! {":task": task, ":task_order": task_order, ":stack_id": current_stack_id})?;
    xact.commit()?;
    Ok(())
}

/// Pop the current task and push it onto `destination_stack`.
fn pop_to(db: &Connection, destination_stack: String) -> Result<(), Box<dyn Error>> {
    let current_stack_id = get_current_stack_id(db)?;
    let destination_stack_id = stack_name_to_id(db, &destination_stack)?;
    let maybe_top_task_id: Option<u32> = db.query_row("SELECT id FROM tasks WHERE task_order = (SELECT max(task_order) FROM tasks WHERE stack_id = :stack_id) WHERE stack_id = :stack_id",
    named_params! {":stack_id": current_stack_id}, |row| row.get(0)).optional()?;
    if let Some(task_id) = maybe_top_task_id {
        db.execute("UPDATE tasks SET stack_id = :stack_id WHERE id = :task_id", named_params! {":stack_id": destination_stack_id, ":task_id": task_id})?;
    }
    Ok(())
}

/// Create a new stack called `stack_name`.
/// 
/// Returns an error if the stack already exists.
fn new_stack(db: &Connection, stack_name: String) -> Result<(), Box<dyn Error>> {
    let stack_exists: Option<i32> = db.query_row("SELECT 1 FROM stacks WHERE name = ?", params![stack_name], |row| row.get(0)).optional()?;
    if let Some(_) = stack_exists {
        return Err(format!("Stack {} already exists!", stack_name).into());
    }

    db.execute("INSERT INTO stacks(name) VALUES (?)", params![stack_name])?;
    Ok(())
}

/// Convert a stack name into an ID.
///
/// Returns an error if `name` does not refer to an existing stack.
fn stack_name_to_id(db: &Connection, name: &str) -> Result<StackId, Box<dyn Error>> {
    let maybe_stack_id: Option<StackId> = db.query_row("SELECT id FROM stacks WHERE name = ?",
        params![name], |row| row.get(0)).optional()?;
    match maybe_stack_id {
        None => return Err(format!("stack '{}' does not exist", name).into()),
        Some(id) => Ok(id)
    }
}

/// Drop a stack and all tasks in it.
fn drop_stack(db: &mut Connection, stack_name: String) -> Result<(), Box<dyn Error>> {
    let current_stack_id = get_current_stack_id(db)?;
    let stack_id = stack_name_to_id(db, &stack_name)?;
    if stack_id == DEFAULT_STACK_ID {
        return Err("Can't delete the default stack".into());
    } else if stack_id == current_stack_id {
        return Err("Can't delete the current stack".into());
    }
    let xact = db.transaction()?;
    xact.execute("DELETE FROM tasks WHERE stack_id = ?", params![stack_id])?;
    xact.execute("DELETE FROM stacks WHERE id = ?", params![stack_id])?;
    xact.commit()?;
    Ok(())
}

fn switch_to_stack(db: &Connection, stack_name: String) -> Result<(), Box<dyn Error>> {
    let stack_id = stack_name_to_id(db, &stack_name)?;
    db.execute("UPDATE app_state SET stack_id = ?", params![stack_id])?;
    Ok(())
}

fn list_stacks(db: &Connection) -> RusqliteResult<Vec<String>> {
    let mut stmt = db.prepare("SELECT name FROM stacks")?;
    let result = stmt.query_map([], |row| row.get(0))?.collect();
    result
}


fn list_tasks(db: &Connection) -> RusqliteResult<Vec<String>> {
    let current_stack_id = get_current_stack_id(db)?;
    let mut stmt = db.prepare("SELECT task FROM tasks WHERE stack_id = ? ORDER BY task_order")?;
    let result = stmt.query_map(params![current_stack_id], |row| row.get(0))?.collect();
    result
}

fn swap_tasks(db: &mut Connection, idx1: u64, idx2: u64) -> Result<(), Box<dyn Error>> {
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

    let (min, max) = (cmp::min(idx1, idx2) + 1, cmp::max(idx1, idx2) + 1);
    let mut data: Vec<(i32, i32)> = Vec::with_capacity(2);
    {
        let mut stmt = db.prepare(
            "SELECT task_order, id FROM (SELECT task_order, id, row_number() OVER (ORDER BY task_order) row FROM tasks) WHERE row IN (?, ?)"
        )?;
        let rows = stmt.query_map(params![min, max], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            data.push(row?);
        }
    }
    let xact = db.transaction()?;
    xact.execute("UPDATE tasks SET task_order = ? WHERE id = ?", params![data[0].0, data[1].1])?;
    xact.execute("UPDATE tasks SET task_order = ? WHERE id = ?", params![data[1].0, data[0].1])?;
    xact.commit()?;

    Ok(())
}
