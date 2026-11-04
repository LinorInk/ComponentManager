//! 元件增删改查：GUI 与 CLI 共用的唯一实现。

use rusqlite::{params, Connection, Row};
use std::time::SystemTime;

use crate::models::{err, norm, norm_str, row_to_component, Component, COLS};

/// 列出库内全部元件（按 id 倒序）
pub fn list_all(conn: &Connection, library_id: i64) -> Result<Vec<Component>, String> {
    let sql = format!("SELECT {COLS} FROM components WHERE library_id = ?1 ORDER BY id DESC");
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let out = stmt
        .query_map(params![library_id], row_to_component)
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(out)
}

/// 在当前库内按名称 / 类别 / 封装 / 位置模糊搜索（大小写不敏感，按名称升序）
pub fn search(
    conn: &Connection,
    library_id: i64,
    keyword: &str,
) -> Result<Vec<Component>, String> {
    let kw = keyword.trim();
    if kw.is_empty() {
        return list_all(conn, library_id);
    }
    let sql = format!(
        "SELECT {COLS} FROM components
         WHERE library_id = :lid
           AND (name LIKE :kw ESCAPE '\\'
             OR category LIKE :kw ESCAPE '\\'
             OR package LIKE :kw ESCAPE '\\'
             OR location LIKE :kw ESCAPE '\\')
         ORDER BY name ASC"
    );
    let like = format!("%{}%", kw.replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let out = stmt
        .query_map(
            rusqlite::named_params! { ":lid": library_id, ":kw": like },
            row_to_component,
        )
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(out)
}

pub fn get_by_id(conn: &Connection, id: i64) -> Result<Component, String> {
    conn.query_row(
        &format!("SELECT {COLS} FROM components WHERE id = ?1"),
        params![id],
        row_to_component,
    )
    .map_err(err)
}

/// 新增元件。名称去空白后不可为空；各文本字段空白按 NULL 存；数量非负。
pub fn insert(conn: &Connection, library_id: i64, c: &Component) -> Result<i64, String> {
    let name = c.name.trim().to_string();
    if name.is_empty() {
        return Err("元件名称不能为空".into());
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
    Ok(conn.last_insert_rowid())
}

/// 全字段更新（GUI 编辑弹窗语义：整体覆盖）
pub fn update_full(conn: &Connection, c: &Component) -> Result<bool, String> {
    let name = c.name.trim().to_string();
    if name.is_empty() {
        return Err("元件名称不能为空".into());
    }
    let changed = conn
        .execute(
            "UPDATE components SET
                name = ?2, category = ?3, package = ?4, manufacturer = ?5,
                quantity = ?6, location = ?7, datasheet_url = ?8, notes = ?9, precision = ?10
             WHERE id = ?1",
            params![
                c.id,
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
    Ok(changed > 0)
}

/// 部分字段更新意图：只改动明确给出的字段，未给出的保持原值。
#[derive(Debug, Default, Clone)]
pub struct ComponentPatch {
    pub name: Option<String>,
    pub category: Option<String>,
    pub package: Option<String>,
    pub manufacturer: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub precision: Option<String>,
    pub quantity: Option<i64>,
    /// 双层 Option：外层表示“是否改动”，内层 Some / None 表示设值 / 置空
    pub datasheet_url: Option<Option<String>>,
}

impl ComponentPatch {
    /// 按列名取文本字段的写入槽，供调用方逐项填值；未知列名返回 None
    pub fn text_mut(&mut self, col: &str) -> Option<&mut Option<String>> {
        match col {
            "name" => Some(&mut self.name),
            "category" => Some(&mut self.category),
            "package" => Some(&mut self.package),
            "manufacturer" => Some(&mut self.manufacturer),
            "location" => Some(&mut self.location),
            "notes" => Some(&mut self.notes),
            "precision" => Some(&mut self.precision),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.category.is_none()
            && self.package.is_none()
            && self.manufacturer.is_none()
            && self.location.is_none()
            && self.notes.is_none()
            && self.precision.is_none()
            && self.quantity.is_none()
            && self.datasheet_url.is_none()
    }
}

/// 按 id + 库 部分更新元件字段，返回更新后的完整记录。
/// 名称去空白后不可为空；文本字段空白按 NULL 存；数量非负。
pub fn apply_patch(
    conn: &Connection,
    library_id: i64,
    id: i64,
    patch: &ComponentPatch,
) -> Result<Component, String> {
    if patch.is_empty() {
        return Err("没有要更新的字段".into());
    }
    let mut sets: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(n) = &patch.name {
        let n = norm_str(n).ok_or("名称不能为空")?;
        push_value(&mut sets, &mut vals, "name", n);
    }
    let text_fields = [
        ("category", &patch.category),
        ("package", &patch.package),
        ("manufacturer", &patch.manufacturer),
        ("location", &patch.location),
        ("notes", &patch.notes),
        ("precision", &patch.precision),
    ];
    for (col, value) in text_fields {
        if let Some(v) = value {
            push_value(&mut sets, &mut vals, col, norm_str(v));
        }
    }
    if let Some(q) = patch.quantity {
        push_value(&mut sets, &mut vals, "quantity", q.max(0));
    }
    if let Some(v) = &patch.datasheet_url {
        let stored = norm_str(v.as_deref().unwrap_or(""));
        push_value(&mut sets, &mut vals, "datasheet_url", stored);
    }

    let sql = format!(
        "UPDATE components SET {} WHERE id = ?{} AND library_id = ?{}",
        sets.join(", "),
        vals.len() + 1,
        vals.len() + 2
    );
    vals.push(Box::new(id));
    vals.push(Box::new(library_id));
    let refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let changed = conn.execute(&sql, refs.as_slice()).map_err(err)?;
    if changed == 0 {
        return Err(format!("未找到元件 id={id}，库 {library_id}"));
    }
    get_by_id(conn, id)
}

/// 按名称增减库存：名称精确匹配（忽略大小写），未命中时给出相近元件提示；
/// 同名多条时提示改用按 id 更新。返回更新后的完整记录。
pub fn adjust_by_name(
    conn: &Connection,
    library_id: i64,
    name: &str,
    delta: i64,
) -> Result<Component, String> {
    let hits = find_by_exact_name(conn, library_id, name)?;
    match hits.len() {
        0 => {
            let cands = search(conn, library_id, name)?;
            if cands.is_empty() {
                return Err(format!("库 {library_id} 中没有元件「{name}」"));
            }
            Err(format!(
                "没有精确匹配「{name}」；相近的有：{}",
                cands
                    .iter()
                    .take(5)
                    .map(|c| format!("id={} {}", c.id, c.name))
                    .collect::<Vec<_>>()
                    .join("；")
            ))
        }
        1 => {
            let id = hits[0].0;
            bump_quantity(conn, id, delta)?;
            get_by_id(conn, id)
        }
        n => Err(format!(
            "库 {library_id} 中有 {n} 个同名元件「{name}」，id：{}，请改用按 id 更新",
            hits.iter()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn delete_by_id(conn: &Connection, id: i64) -> Result<bool, String> {
    let changed = conn
        .execute("DELETE FROM components WHERE id = ?1", params![id])
        .map_err(err)?;
    Ok(changed > 0)
}

/// 带库校验的删除（CLI 语义：id 必须属于该库）
pub fn delete_in_library(conn: &Connection, id: i64, library_id: i64) -> Result<bool, String> {
    let changed = conn
        .execute(
            "DELETE FROM components WHERE id = ?1 AND library_id = ?2",
            params![id, library_id],
        )
        .map_err(err)?;
    Ok(changed > 0)
}

/// 按名称精确匹配（忽略大小写）查找库内元件，返回 (id, 当前数量)
pub fn find_by_exact_name(
    conn: &Connection,
    library_id: i64,
    name: &str,
) -> Result<Vec<(i64, i64)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, quantity FROM components WHERE library_id = ?1 AND LOWER(name) = LOWER(?2)")
        .map_err(err)?;
    let out = stmt
        .query_map(params![library_id, name], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(out)
}

/// 库存增减，结果下限 0
pub fn bump_quantity(conn: &Connection, id: i64, delta: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE components SET quantity = MAX(0, quantity + ?1) WHERE id = ?2",
        params![delta, id],
    )
    .map_err(err)?;
    Ok(())
}

/// 库内当前本地时间（导出文件时间戳用）
pub fn now_local(conn: &Connection) -> Result<String, String> {
    conn.query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(err)
}

/// 供部分字段更新拼装动态 SQL 用的辅助：追加一组 SET 值
pub fn push_value(
    sets: &mut Vec<String>,
    vals: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    col: &str,
    v: impl rusqlite::types::ToSql + 'static,
) {
    let idx = vals.len() + 1;
    sets.push(format!("{} = ?{}", col, idx));
    vals.push(Box::new(v));
}

pub fn as_str_row(r: &Row<'_>, idx: usize) -> rusqlite::Result<String> {
    r.get(idx)
}

/// 文件修改时间的展示（UNIX 秒 → UTC+8 大致时间，展示用足够，不引额外时间库）
pub fn mtime_of(path: &std::path::Path) -> String {
    let Ok(t) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return String::new();
    };
    const TZ_OFFSET_SECS: i64 = 8 * 3600;
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + TZ_OFFSET_SECS;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02}", rem / 3600 % 24, rem / 60 % 60)
}

/// Howard Hinnant civil_from_days（公历 proleptic）
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}
