use crate::types::*;
use crate::errors::*;

use std::cmp;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::path::PathBuf;
use std::env;

use regex::Regex;
use rusqlite::Transaction;
use rusqlite::{Connection, params, named_params, OptionalExtension};
use rusqlite::Result as RusqliteResult;
use notify_rust::Notification;
use uuid::Uuid;

/// Get the ID of the current stack.
pub fn get_current_stack_id(db: &Connection) -> AppResult<StackId> {
    let stack_id: StackId = db.query_row("SELECT stack_id FROM app_state", [], |row| row.get(0))?;
    Ok(stack_id)
}

/// Get the name of the current stack.
pub fn get_current_stack_name(db: &Connection) -> AppResult<String> {
    let current_stack_id = get_current_stack_id(db)?;
    let current_stack_name: String = db.query_row("SELECT name FROM stacks WHERE id = ?", params![current_stack_id], |row| row.get(0))?;
    Ok(current_stack_name)
}

/// Push `task` onto the top of the stack.
pub fn push_task(db: &Connection, task: String) -> AppResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (?, (SELECT coalesce(max(task_order) + 1, 1) FROM tasks), ?)", params![task, current_stack_id])?;
    Ok(())
}

/// Put `task` onto the bottom of the stack.
pub fn pushback_task(db: &Connection, task: String) -> AppResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (?, (SELECT coalesce(min(task_order) - 1, 1) FROM tasks), ?)", params![task, current_stack_id])?;
    Ok(())
}

/// Pop the top task off the stack.
pub fn pop_task(db: &Connection) -> AppResult<Option<String>> {
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

/// Clear all tasks from the current stack.
pub fn clear_tasks(db: &Connection) -> AppResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    db.execute("DELETE FROM tasks WHERE stack_id = ?", params![current_stack_id])?;
    Ok(())
}

/// Clear all tasks from all stacks.
pub fn clear_all_tasks(db: &Connection) -> AppResult<()> {
    db.execute("DELETE FROM tasks WHERE 1 = 1", [])?;
    Ok(())
}

/// Insert `task` after the `task_index`th task, starting from 0.
/// 
/// i.e. if `task_index == 0`, then this is equivalent to `backpush`
fn insert_after(db: &mut Connection, task_index: TaskIndex, task: String) -> AppResult<()> {
    // two cases: task is last and task is not last
    // if task is not last, avg() works
    // if task is last, avg() just gives task order
    let current_stack_id = get_current_stack_id(db)?;
    let num_tasks = db.query_row("SELECT count(*) FROM tasks WHERE stack_id = ?", params![current_stack_id], |row| row.get(0))?;
    if task_index >= num_tasks {
        return Err(TaskError::NoSuchTask(task_index).into());
    } else if task_index == 0 {
        return Ok(push_task(db, task)?);
    } else if task_index == num_tasks - 1 {
        return Ok(pushback_task(db, task)?);
    }

    // sqlite starts rows from 1
    let task_index = task_index + 1;
    // task is not last, we are good to go.
    let task_order: i64 = db.query_row("SELECT task_order + 1 FROM (SELECT row_number() OVER (ORDER BY task_order) task_index, task_order FROM tasks) WHERE task_index = :task_index", 
    named_params! {":task_index": task_index}, |row| row.get(0))?;
    let xact = db.transaction()?;
    xact.execute("UPDATE tasks SET task_order = task_order + 1 WHERE task_order >= :task_order AND stack_id = :stack_id", named_params! {":task_order": task_order, ":stack_id": current_stack_id})?;
    xact.execute("INSERT INTO tasks(task, task_order, stack_id) VALUES (:task, :task_order, :stack_id)", named_params! {":task": task, ":task_order": task_order, ":stack_id": current_stack_id})?;
    xact.commit()?;
    Ok(())
}

/// Pop the current task and push it onto `destination_stack`.
pub fn pop_to(db: &Connection, destination_stack: String) -> AppResult<()> {
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
pub fn new_stack(db: &Connection, stack_name: String) -> AppResult<()> {
    let stack_exists: Option<i32> = db.query_row("SELECT 1 FROM stacks WHERE name = ?", params![stack_name], |row| row.get(0)).optional()?;
    if let Some(_) = stack_exists {
        return Err(StackError::StackAlreadyExists(stack_name).into());
    }

    db.execute("INSERT INTO stacks(name) VALUES (?)", params![stack_name])?;
    Ok(())
}

/// Convert a stack name into an ID.
///
/// Returns an error if `name` does not refer to an existing stack.
fn stack_name_to_id(db: &Connection, name: &str) -> AppResult<StackId> {
    let maybe_stack_id: Option<StackId> = db.query_row("SELECT id FROM stacks WHERE name = ?",
        params![name], |row| row.get(0)).optional()?;
    match maybe_stack_id {
        None => return Err(StackError::NoSuchStack(name.into()).into()),
        Some(id) => Ok(id)
    }
}

/// Drop a stack and all tasks in it.
pub fn drop_stack(db: &mut Connection, stack_name: String) -> AppResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    let stack_id = stack_name_to_id(db, &stack_name)?;
    if stack_id == DEFAULT_STACK_ID {
        return Err(StackError::CantDeleteDefaultStack.into());
    } else if stack_id == current_stack_id {
        return Err(StackError::CantDeleteCurrentStack.into());
    }
    let xact = db.transaction()?;
    xact.execute("DELETE FROM tasks WHERE stack_id = ?", params![stack_id])?;
    xact.execute("DELETE FROM stacks WHERE id = ?", params![stack_id])?;
    xact.commit()?;
    Ok(())
}

/// Switch to the stack `stack_name`.
pub fn switch_to_stack(db: &Connection, stack_name: String) -> AppResult<()> {
    let stack_id = stack_name_to_id(db, &stack_name)?;
    db.execute("UPDATE app_state SET stack_id = ?", params![stack_id])?;
    Ok(())
}

/// List all stacks.
pub fn list_stacks(db: &Connection) -> RusqliteResult<Vec<String>> {
    let mut stmt = db.prepare("SELECT name FROM stacks")?;
    let result = stmt.query_map([], |row| row.get(0))?.collect();
    result
}


pub fn list_tasks(db: &Connection) -> AppResult<Vec<String>> {
    let current_stack_id = get_current_stack_id(db)?;
    let mut stmt = db.prepare("SELECT task FROM tasks WHERE stack_id = ? ORDER BY task_order")?;
    let mut tasks = Vec::new();
    let rows = stmt.query_map(params![current_stack_id], |row| row.get(0))?;
    for row in rows {
        tasks.push(row?);
    }
    Ok(tasks)
}

pub fn swap_tasks(db: &mut Connection, idx1: TaskIndex, idx2: TaskIndex) -> AppResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    let task_count: TaskIndex = db.query_row("SELECT count(*) FROM tasks WHERE stack_id = ?", params![current_stack_id], |row| row.get(0))?;
    match (idx1 >= task_count, idx2 >= task_count) {
        (false, false) => {}
        (true, false) | (false, true) => {
            let invalid = cmp::max(idx1, idx2);
            return Err(TaskError::NoSuchTask(invalid).into());
        }
        (true, true) => {
            return Err(TaskError::NoSuchTasks(idx1, idx2).into());
        }
    }

    let (min, max) = (cmp::min(idx1, idx2), cmp::max(idx1, idx2));
    let min_id = task_index_to_task_id(db, current_stack_id, min)?;
    let max_id = task_index_to_task_id(db, current_stack_id, max)?;
    let min_order: i32 = db.query_row("SELECT task_order FROM tasks WHERE stack_id = ? AND id = ?", params![current_stack_id, min_id], |r| r.get(0))?;
    let max_order: i32 = db.query_row("SELECT task_order FROM tasks WHERE stack_id = ? AND id = ?", params![current_stack_id, max_id], |r| r.get(0))?;
    let xact = db.transaction()?;
    xact.execute("UPDATE tasks SET task_order = ? WHERE id = ?", params![max_order, min_id])?;
    xact.execute("UPDATE tasks SET task_order = ? WHERE id = ?", params![min_order, max_id])?;
    xact.commit()?;

    Ok(())
}

fn task_index_to_task_id(db: &mut Connection, stack_id: StackId, task_index: TaskIndex) -> AppResult<i32> {
    let task_count: TaskIndex = db.query_row("SELECT count(*) FROM tasks WHERE stack_id = ?", params![stack_id], |row| row.get(0))?;
    if task_index >= task_count {
        return Err(TaskError::NoSuchTask(task_index).into());
    }

    let id = db.query_row("SELECT id FROM (SELECT id, row_number() OVER (ORDER BY task_order) row FROM tasks WHERE stack_id = ?) WHERE row = (? + 1)",
    params![stack_id, task_index], 
    |row| row.get(0))?;
    Ok(id)
}

pub fn kill_task(db: &mut Connection, idx: TaskIndex) -> AppResult<String> {
    let current_stack_id = get_current_stack_id(db)?;
    let task_count: TaskIndex = db.query_row("SELECT count(*) FROM tasks WHERE stack_id = ?", params![current_stack_id], |row| row.get(0))?;
    if idx >= task_count {
        return Err(TaskError::NoSuchTask(idx).into());
    }
    let task_id = task_index_to_task_id(db, current_stack_id, idx)?;
    let task_description = db.query_row("SELECT task FROM tasks WHERE stack_id = ? AND id = ?", params![current_stack_id, task_id], |row| row.get(0))?;
    db.execute("DELETE FROM tasks WHERE stack_id = ? AND id = ?", params![current_stack_id, task_id])?;

    Ok(task_description)
}

fn parse_delay_spec_into_seconds(spec: &str) -> AppResult<u32> {
    let spec_regex = Regex::new("(?P<amount>[1-9][0-9]{0,5})(?P<unit>[hms])").expect("bug: invalid regex in parse_delay_spec");
    if !spec_regex.is_match(spec) {
        return Err(ReminderError::InvalidReminderTime(spec.into()).into());
    }
    let caps = spec_regex.captures(spec).expect("bug: delay spec regex matches but cannot get captures");
    let amount: u32 = caps.name("amount")
        .expect("amount capture missing from delay spec regex")
        .as_str()
        .parse()
        .expect("invalid number in delay spec");
    let unit = caps.name("unit")
        .expect("unit capture missing from delay spec regex")
        .as_str();
    let multiplier = match unit {
        "h" => 60*60,
        "m" => 60,
        "s" => 1,
        _ => unreachable!("Invalid unit: {}", unit)
    };
    Ok(amount.checked_mul(multiplier).expect("bug: overflow in delay time"))
}

pub fn remind_me(db: &mut Connection, task_index: TaskIndex, reminder_string: String) -> AppResult<()> {
    let current_stack_id = get_current_stack_id(db)?;
    let task_id = task_index_to_task_id(db, current_stack_id, task_index)?;
    let delay_time = parse_delay_spec_into_seconds(&reminder_string)?;
    let current_bin = env::current_exe().map_err(|e| AppError::Environment(format!("unable to obtain path to current executable: {}", e)))?;
    // Lock the entire DB to prevent any other modifications
    let xact = Transaction::new(db, rusqlite::TransactionBehavior::Exclusive)?;
    let reminder_id = Uuid::new_v4().to_string();
    xact.execute("INSERT INTO reminders(id, delay, task_id) VALUES (?, ?, ?)", params![reminder_id, delay_time, task_id])?;
    // Potential race condition: We spawn the command before committing the transaction.
    // To ensure this does not cause issues, lock the whole database (using an exclusive xact).
    Command::new(current_bin)
        .arg("triggerreminder")
        .arg(format!("{}", reminder_id))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AppError::Environment(format!("unable to spawn reminder process: {}", e)))?;
    xact.commit()?;
    // Do not wait on the process; let it run in the background
    Ok(())
}


pub fn trigger_reminder(db_path: PathBuf, db: Connection, reminder_id: String) -> AppResult<()> {
    let (reminder_delay, task_id): (u32, i64) = db.query_row("SELECT delay, task_id FROM reminders WHERE id = ?", params![reminder_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    // Close the DB connection, we don't want to hold onto it while waiting.
    if let Err((_, e)) = db.close() {
        return Err(e.into());
    }
    thread::sleep(Duration::from_secs(reminder_delay as u64));
    let mut db = Connection::open(db_path)?;
    let xact = db.transaction()?;
    let task: String = xact.query_row("SELECT task FROM tasks WHERE id = ?", params![task_id], |row| row.get(0))?;
    xact.execute("DELETE FROM reminders WHERE id = ?", params![reminder_id])?;
    xact.commit()?;

    Notification::new()
        .summary("Task Reminder")
        .body(&task)
        .timeout(10_000)
        .show()
        .expect("Failed to show notification");
    Ok(())
}