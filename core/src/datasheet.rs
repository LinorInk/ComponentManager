//! 本地数据手册文件的存储、列表与删除：GUI 与 CLI 共用。
//! 手册统一存放在数据目录下的 datasheets 文件夹；库内 datasheet_url 字段
//! 对本地文件存裸文件名，对在线链接原样存 URL。

use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;

use crate::components::mtime_of;
use crate::models::{err, DatasheetImport, DatasheetInfo};

/// 手册文件存放目录：数据目录下的 datasheets 文件夹
pub fn dir_in(data_dir: &Path) -> Result<std::path::PathBuf, String> {
    let dir = data_dir.join("datasheets");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 校验裸文件名，防目录穿越
fn check_bare_name(file_name: &str) -> Result<(), String> {
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") || file_name.is_empty() {
        return Err("非法的手册文件名".into());
    }
    Ok(())
}

/// 把选中的本地手册复制进 datasheets 目录。
/// 同名文件跳过复制、直接复用已有的那份，由调用方弹窗提醒。
pub fn import_file(data_dir: &Path, src: &str) -> Result<DatasheetImport, String> {
    let src_path = Path::new(src);
    if !src_path.is_file() {
        return Err("所选文件不存在".into());
    }
    let dir = dir_in(data_dir)?;
    let file_name = src_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "datasheet.pdf".into());
    let target = dir.join(&file_name);
    if target.exists() {
        return Ok(DatasheetImport { file_name, reused: true });
    }
    fs::copy(src_path, &target).map_err(err)?;
    Ok(DatasheetImport { file_name, reused: false })
}

/// CLI --datasheet 参数语义：http(s) 链接原样存；本地文件路径复制入库后返回裸文件名
pub fn store_value(data_dir: &Path, value: &str) -> Result<String, String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Ok(value.to_string());
    }
    let res = import_file(data_dir, value)?;
    Ok(res.file_name)
}

/// 手册文件在数据目录内的完整路径（含存在性校验）
pub fn resolve(data_dir: &Path, file_name: &str) -> Result<std::path::PathBuf, String> {
    check_bare_name(file_name)?;
    let target = dir_in(data_dir)?.join(file_name);
    if !target.is_file() {
        return Err(format!("手册文件不存在：{file_name}"));
    }
    Ok(target)
}

/// 扫描 datasheets 目录，附带每个文件被哪些库的哪些元件引用
pub fn list_with_refs(data_dir: &Path, conn: &Connection) -> Result<Vec<DatasheetInfo>, String> {
    let dir = dir_in(data_dir)?;
    let entries = fs::read_dir(&dir).map_err(err)?;

    let mut out: Vec<DatasheetInfo> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let refs = references(conn, &file_name)?;
        out.push(DatasheetInfo {
            file_name,
            size,
            mtime: mtime_of(&path),
            refs,
        });
    }
    out.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    Ok(out)
}

/// 引用查询：datasheet_url 精确等于该文件名的元件，返回“库名 / 元件名”列表
pub fn references(conn: &Connection, file_name: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.name, c.name FROM components c
             JOIN libraries l ON l.id = c.library_id
             WHERE c.datasheet_url = ?1
             ORDER BY l.name, c.name",
        )
        .map_err(err)?;
    let out = stmt
        .query_map(params![file_name], |row| {
            let lib: String = row.get(0)?;
            let comp: String = row.get(1)?;
            Ok(format!("{lib} / {comp}"))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(out)
}

/// 删除手册文件；若仍被引用，同步清除引用元件的 datasheet_url。返回清除的引用数。
pub fn delete(data_dir: &Path, conn: &Connection, file_name: &str) -> Result<i64, String> {
    check_bare_name(file_name)?;
    let target = dir_in(data_dir)?.join(file_name);
    if !target.is_file() {
        return Err(format!("文件不存在：{file_name}"));
    }
    let cleared = conn
        .execute(
            "UPDATE components SET datasheet_url = NULL WHERE datasheet_url = ?1",
            params![file_name],
        )
        .map_err(err)?;
    fs::remove_file(&target).map_err(err)?;
    Ok(cleared as i64)
}
