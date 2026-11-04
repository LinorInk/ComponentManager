//! 数据库打开与迁移入口：GUI 与 CLI 共用同一份连接建立逻辑。

use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

use crate::models::{err, migrate};

/// 打开（或创建）指定路径的数据库：等待其他进程释放锁，然后建表与迁移。
/// GUI 与 CLI 可能同时开着库，busy_timeout 让后到的操作等锁而不是直接报错。
pub fn open_at(file: &Path) -> Result<Connection, String> {
    let conn = Connection::open(file).map_err(err)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(err)?;
    migrate(&conn).map_err(err)?;
    Ok(conn)
}
