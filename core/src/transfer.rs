//! 库导入导出（JSON 分享文件）：GUI 与 CLI 共用的唯一格式与读写实现。

use rusqlite::{params, Connection};

use crate::components;
use crate::models::{err, norm, Component, ImportResult, LibraryFile, FILE_APP};

/// 组装当前库的分享文件内容
pub fn build_file(conn: &Connection, library_id: i64) -> Result<LibraryFile, String> {
    let lib_name = crate::models::require_library(conn, library_id)?;
    let comps = components::list_all(conn, library_id)?;
    let now = components::now_local(conn)?;
    Ok(LibraryFile {
        app: FILE_APP.to_string(),
        version: 1,
        library: lib_name,
        exported_at: now,
        components: comps,
    })
}

/// 解析并校验分享文件文本
pub fn parse_file(text: &str) -> Result<LibraryFile, String> {
    let file: LibraryFile = serde_json::from_str(text)
        .map_err(|e| format!("文件解析失败，不是有效的导出文件：{e}"))?;
    if file.app != FILE_APP {
        return Err("该文件不是元件库存管理器导出的文件".into());
    }
    Ok(file)
}

/// 把分享文件中的元件合并进目标库；与库内重名（忽略大小写）的条目跳过
pub fn merge_into(conn: &Connection, library_id: i64, file: &LibraryFile) -> Result<ImportResult, String> {
    let mut added = 0i64;
    let mut skipped = 0i64;
    for c in &file.components {
        let name = c.name.trim().to_string();
        if name.is_empty() {
            skipped += 1;
            continue;
        }
        let dup: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM components WHERE library_id = ?1 AND LOWER(name) = LOWER(?2)",
                params![library_id, name],
                |r| r.get(0),
            )
            .map_err(err)?;
        if dup > 0 {
            skipped += 1;
            continue;
        }
        conn.execute(
            "INSERT INTO components
                (library_id, name, category, package, manufacturer, quantity, location, datasheet_url, notes, precision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                library_id,
                name,
                norm(&c.category),
                norm(&c.package),
                norm(&c.manufacturer),
                c.quantity.max(0),
                norm(&c.location),
                norm(&c.datasheet_url),
                norm(&c.notes),
                norm(&c.precision),
            ],
        )
        .map_err(err)?;
        added += 1;
    }
    Ok(ImportResult { added, skipped })
}

/// 序列化为美化 JSON 文本（写盘用）
pub fn to_pretty(file: &LibraryFile) -> Result<String, String> {
    serde_json::to_string_pretty(file).map_err(err)
}

pub fn component_count(file: &LibraryFile) -> usize {
    file.components.len()
}

pub fn components_of(file: &LibraryFile) -> &Vec<Component> {
    &file.components
}
