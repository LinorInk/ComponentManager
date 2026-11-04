//! 数据目录与数据库入口：CLI 侧的目录约定只在这里写一次。
//! 数据落在 %APPDATA%\com.componentmanager.app（与图形界面一致）。
//! 测试可用环境变量 CM_DB_PATH 覆盖数据库文件路径。

use rusqlite::Connection;
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::db;
use crate::models::err;

/// 应用数据目录名
pub const APP_DIR_NAME: &str = "com.componentmanager.app";
/// 数据库文件名
pub const DB_FILE_NAME: &str = "components.db";
/// 覆盖数据库路径的环境变量
pub const DB_PATH_ENV: &str = "CM_DB_PATH";

/// 数据目录（手册文件、数据库都放这里）；CM_DB_PATH 生效时取该文件所在目录
pub fn data_dir() -> Result<PathBuf, String> {
    if let Ok(p) = env::var(DB_PATH_ENV) {
        let dir = PathBuf::from(p)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&dir).map_err(err)?;
        return Ok(dir);
    }
    let appdata = env::var("APPDATA").map_err(|_| "未找到 APPDATA 环境变量".to_string())?;
    let dir = PathBuf::from(appdata).join(APP_DIR_NAME);
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 数据库文件路径
pub fn db_file() -> Result<PathBuf, String> {
    if let Ok(p) = env::var(DB_PATH_ENV) {
        return Ok(PathBuf::from(p));
    }
    Ok(data_dir()?.join(DB_FILE_NAME))
}

/// 打开数据库：建表、老库补列迁移、等锁，全部由 db::open_at 统一完成
pub fn open_db() -> Result<Connection, String> {
    db::open_at(&db_file()?)
}
