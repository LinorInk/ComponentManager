// cmcli —— 元件库存管理器命令行工具
// 与图形界面（main.rs）共用同一个 SQLite 数据库，供脚本 / AI 快速录入与查询元件。
// 用法见根目录《命令行使用说明.md》或运行 `cmcli help`。

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// 必须与 main.rs 中的表结构保持一致
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS libraries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS components (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    library_id INTEGER NOT NULL DEFAULT 1,
    name TEXT NOT NULL,
    category TEXT,
    package TEXT,
    manufacturer TEXT,
    quantity INTEGER DEFAULT 0,
    location TEXT,
    datasheet_url TEXT,
    notes TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_components_name ON components(name);
CREATE INDEX IF NOT EXISTS idx_components_category ON components(category);
CREATE INDEX IF NOT EXISTS idx_components_library ON components(library_id);
"#;

const COLS: &str =
    "id, name, category, package, manufacturer, quantity, location, datasheet_url, notes, created_at";

/// 与图形界面导出/导入文件的标识一致
const APP_TAG: &str = "ComponentManager";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Component {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    quantity: i64,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    datasheet_url: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LibraryFile {
    app: String,
    version: u32,
    library: String,
    exported_at: String,
    components: Vec<Component>,
}

#[derive(Debug, Serialize)]
struct LibraryRow {
    id: i64,
    name: String,
    count: i64,
}

// ---------------- 基础设施 ----------------

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn norm(v: &str) -> Option<String> {
    let t = v.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn row_to_component(row: &Row) -> rusqlite::Result<Component> {
    Ok(Component {
        id: row.get(0)?,
        name: row.get(1)?,
        category: row.get(2)?,
        package: row.get(3)?,
        manufacturer: row.get(4)?,
        quantity: row.get(5)?,
        location: row.get(6)?,
        datasheet_url: row.get(7)?,
        notes: row.get(8)?,
        created_at: row.get(9)?,
    })
}

/// 数据库位置与 GUI 相同：%APPDATA%\com.componentmanager.app\components.db
/// 测试可用环境变量 CM_DB_PATH 覆盖
fn data_dir() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("CM_DB_PATH") {
        let dir = PathBuf::from(p)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&dir).map_err(err)?;
        return Ok(dir);
    }
    let appdata = env::var("APPDATA").map_err(|_| "未找到 APPDATA 环境变量".to_string())?;
    let dir = PathBuf::from(appdata).join("com.componentmanager.app");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

fn db_file() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("CM_DB_PATH") {
        return Ok(PathBuf::from(p));
    }
    Ok(data_dir()?.join("components.db"))
}

fn open_db() -> Result<Connection, String> {
    let file = db_file()?;
    let conn = Connection::open(&file).map_err(err)?;
    // GUI 可能同时开着库，等待锁而不是直接报错
    conn.busy_timeout(Duration::from_secs(5)).map_err(err)?;
    conn.execute_batch(SCHEMA).map_err(err)?;
    Ok(conn)
}

fn require_library(conn: &Connection, id: i64) -> Result<String, String> {
    conn
        .query_row(
            "SELECT name FROM libraries WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| format!("元件库不存在：id={id}（先用 cmcli libs 查看可用库）"))
}

fn datasheet_dir() -> Result<PathBuf, String> {
    let dir = data_dir()?.join("datasheets");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 手册字段值：http(s) 链接原样存；本地文件复制进 datasheets（同名复用），返回最终存库的值
fn store_datasheet(value: &str) -> Result<String, String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Ok(value.to_string());
    }
    let src = PathBuf::from(value);
    if !src.is_file() {
        return Err(format!("手册文件不存在：{value}"));
    }
    let file_name = src
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| "无法取得文件名".to_string())?;
    let target = datasheet_dir()?.join(&file_name);
    if !target.exists() {
        fs::copy(&src, &target).map_err(err)?;
    }
    Ok(file_name)
}

fn print_json<T: Serialize>(v: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).expect("JSON 序列化失败")
    );
}

// ---------------- 参数解析 ----------------

struct Parsed {
    cmd: String,
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

fn parse_args() -> Result<Parsed, String> {
    let raw: Vec<String> = env::args().skip(1).collect();
    if raw.is_empty() || raw[0] == "help" || raw[0] == "--help" || raw[0] == "-h" {
        return Err("USAGE".into()); // 主函数里转成帮助文本
    }
    let mut p = Parsed {
        cmd: raw[0].clone(),
        positional: Vec::new(),
        flags: HashMap::new(),
    };
    let mut i = 1;
    while i < raw.len() {
        let tok = &raw[i];
        if let Some(key) = tok.strip_prefix("--") {
            let val = raw
                .get(i + 1)
                .ok_or_else(|| format!("缺少 --{key} 的值"))?;
            p.flags.insert(key.to_string(), val.clone());
            i += 2;
        } else {
            p.positional.push(tok.clone());
            i += 1;
        }
    }
    Ok(p)
}

fn need_str(p: &Parsed, idx: usize, what: &str) -> Result<String, String> {
    p.positional
        .get(idx)
        .cloned()
        .ok_or_else(|| format!("缺少参数：{what}"))
}

fn need_id(p: &Parsed, idx: usize) -> Result<i64, String> {
    let s = need_str(p, idx, "库 id")?;
    s.parse::<i64>().map_err(|_| format!("库 id 必须是数字，收到：{s}"))
}

// ---------------- 子命令 ----------------

fn cmd_libs(p: &Parsed) -> Result<(), String> {
    if !p.positional.is_empty() {
        return Err("用法：cmcli libs".into());
    }
    let conn = open_db()?;
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.name,
                    (SELECT COUNT(*) FROM components c WHERE c.library_id = l.id)
             FROM libraries l ORDER BY l.id",
        )
        .map_err(err)?;
    let out = stmt
        .query_map([], |r| {
            Ok(LibraryRow {
                id: r.get(0)?,
                name: r.get(1)?,
                count: r.get(2)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    print_json(&out);
    Ok(())
}

fn cmd_lib_create(p: &Parsed) -> Result<(), String> {
    let name = need_str(p, 0, "库名")?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("库名不能为空".into());
    }
    let conn = open_db()?;
    conn.execute("INSERT INTO libraries (name) VALUES (?1)", params![name])
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                format!("库名「{name}」已存在")
            } else {
                err(e)
            }
        })?;
    print_json(&serde_json::json!({ "id": conn.last_insert_rowid(), "name": name }));
    Ok(())
}

fn select_components(
    conn: &Connection,
    library_id: i64,
    keyword: Option<&str>,
) -> Result<Vec<Component>, String> {
    let out = match keyword {
        None => {
            let sql =
                format!("SELECT {COLS} FROM components WHERE library_id = ?1 ORDER BY id DESC");
            let mut stmt = conn.prepare(&sql).map_err(err)?;
            let v = stmt
                .query_map(params![library_id], row_to_component)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            v
        }
        Some(kw) => {
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
            let v = stmt
                .query_map(
                    rusqlite::named_params! { ":lid": library_id, ":kw": like },
                    row_to_component,
                )
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            v
        }
    };
    Ok(out)
}

fn cmd_list(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let conn = open_db()?;
    require_library(&conn, lib)?;
    print_json(&select_components(&conn, lib, None)?);
    Ok(())
}

fn cmd_find(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let kw = need_str(p, 1, "关键词")?;
    let conn = open_db()?;
    require_library(&conn, lib)?;
    print_json(&select_components(&conn, lib, Some(&kw))?);
    Ok(())
}

fn cmd_add(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let name = need_str(p, 1, "元件名称")?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("元件名称不能为空".into());
    }
    let qty: i64 = match p.flags.get("qty") {
        Some(v) => v.parse().map_err(|_| format!("--qty 必须是整数，收到：{v}"))?,
        None => 0,
    };
    let datasheet = match p.flags.get("datasheet") {
        Some(v) => Some(store_datasheet(v)?),
        None => None,
    };

    let conn = open_db()?;
    require_library(&conn, lib)?;
    conn.execute(
        "INSERT INTO components
            (library_id, name, category, package, manufacturer, quantity, location, datasheet_url, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            lib,
            name,
            p.flags.get("category").map(|s| norm(s)).flatten(),
            p.flags.get("package").map(|s| norm(s)).flatten(),
            p.flags.get("manufacturer").map(|s| norm(s)).flatten(),
            qty.max(0),
            p.flags.get("location").map(|s| norm(s)).flatten(),
            datasheet,
            p.flags.get("notes").map(|s| norm(s)).flatten(),
        ],
    )
    .map_err(err)?;
    let id = conn.last_insert_rowid();
    let comp = conn
        .query_row(
            &format!("SELECT {COLS} FROM components WHERE id = ?1"),
            params![id],
            row_to_component,
        )
        .map_err(err)?;
    print_json(&comp);
    Ok(())
}

fn cmd_update(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let comp_id: i64 = need_str(p, 1, "元件 id")?
        .parse()
        .map_err(|s| format!("元件 id 必须是数字：{s}"))?;

    let mut sets: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    macro_rules! push_str {
        ($col:expr, $v:expr) => {{
            let idx = vals.len() + 1;
            sets.push(format!("{} = ?{}", $col, idx));
            vals.push(Box::new($v));
        }};
    }

    for key in ["category", "package", "manufacturer", "location", "notes"] {
        if let Some(v) = p.flags.get(key) {
            let v = norm(v).unwrap_or_default();
            push_str!(key, v);
        }
    }
    if let Some(v) = p.flags.get("name") {
        let n = norm(v).ok_or("名称不能为空")?;
        push_str!("name", n);
    }
    if let Some(v) = p.flags.get("qty") {
        let q: i64 = v.parse().map_err(|_| format!("--qty 必须是整数：{v}"))?;
        let idx = vals.len() + 1;
        sets.push(format!("quantity = ?{}", idx));
        vals.push(Box::new(q.max(0)));
    }
    if let Some(v) = p.flags.get("datasheet") {
        let stored = store_datasheet(v)?;
        push_str!("datasheet_url", stored);
    }
    if sets.is_empty() {
        return Err("没有要更新的字段（--name/--category/--package/--manufacturer/--qty/--location/--datasheet/--notes）".into());
    }

    let conn = open_db()?;
    require_library(&conn, lib)?;
    let sql = format!(
        "UPDATE components SET {} WHERE id = ?{} AND library_id = ?{}",
        sets.join(", "),
        vals.len() + 1,
        vals.len() + 2
    );
    vals.push(Box::new(comp_id));
    vals.push(Box::new(lib));
    let refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let changed = conn.execute(&sql, refs.as_slice()).map_err(err)?;
    if changed == 0 {
        return Err(format!(
            "未找到元件 id={comp_id}（确认它属于库 {lib}；cmcli find 可查询）"
        ));
    }
    let comp = conn
        .query_row(
            &format!("SELECT {COLS} FROM components WHERE id = ?1"),
            params![comp_id],
            row_to_component,
        )
        .map_err(err)?;
    print_json(&comp);
    Ok(())
}

/// 库存增减：cmcli adjust <库id> <元件名> <±数量>（按名称精确匹配，忽略大小写）
fn cmd_adjust(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let name = need_str(p, 1, "元件名称")?;
    let delta: i64 = need_str(p, 2, "增减数量")?
        .parse()
        .map_err(|s| format!("数量必须是整数，如 +20 或 -3，收到：{s}"))?;

    let conn = open_db()?;
    require_library(&conn, lib)?;
    let mut stmt = conn
        .prepare("SELECT id, quantity FROM components WHERE library_id = ?1 AND LOWER(name) = LOWER(?2)")
        .map_err(err)?;
    let hits = stmt
        .query_map(params![lib, name], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;

    match hits.len() {
        0 => {
            let cands = select_components(&conn, lib, Some(&name))?;
            if cands.is_empty() {
                Err(format!("库 {lib} 中没有元件「{name}」"))
            } else {
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
        }
        1 => {
            let (id, _q) = hits[0];
            conn.execute(
                "UPDATE components SET quantity = MAX(0, quantity + ?1) WHERE id = ?2",
                params![delta, id],
            )
            .map_err(err)?;
            let comp = conn
                .query_row(
                    &format!("SELECT {COLS} FROM components WHERE id = ?1"),
                    params![id],
                    row_to_component,
                )
                .map_err(err)?;
            print_json(&comp);
            Ok(())
        }
        n => Err(format!(
            "库 {lib} 中有 {n} 个同名元件「{name}」（id：{}），请改用 cmcli update 按 id 操作",
            hits.iter()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn cmd_delete(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let comp_id: i64 = need_str(p, 1, "元件 id")?
        .parse()
        .map_err(|s| format!("元件 id 必须是数字：{s}"))?;
    let conn = open_db()?;
    require_library(&conn, lib)?;
    let changed = conn
        .execute(
            "DELETE FROM components WHERE id = ?1 AND library_id = ?2",
            params![comp_id, lib],
        )
        .map_err(err)?;
    if changed == 0 {
        return Err(format!("未找到元件 id={comp_id}（库 {lib}）"));
    }
    print_json(&serde_json::json!({ "deleted": comp_id }));
    Ok(())
}

fn cmd_export(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let out = need_str(p, 1, "输出文件路径")?;
    let conn = open_db()?;
    let lib_name = require_library(&conn, lib)?;
    let comps = select_components(&conn, lib, None)?;
    let now: String = conn
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(err)?;
    let file = LibraryFile {
        app: APP_TAG.to_string(),
        version: 1,
        library: lib_name,
        exported_at: now,
        components: comps,
    };
    fs::write(&out, serde_json::to_string_pretty(&file).map_err(err)?).map_err(err)?;
    print_json(&serde_json::json!({ "exported": out, "count": file.components.len() }));
    Ok(())
}

fn cmd_import(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let path = need_str(p, 1, "导入文件路径")?;
    let text = fs::read_to_string(&path).map_err(err)?;
    let file: LibraryFile = serde_json::from_str(&text)
        .map_err(|e| format!("文件解析失败，不是有效的导出文件：{e}"))?;
    if file.app != APP_TAG {
        return Err("该文件不是元件库存管理器导出的文件".into());
    }
    let conn = open_db()?;
    require_library(&conn, lib)?;
    let (mut added, mut skipped) = (0i64, 0i64);
    for c in &file.components {
        let name = c.name.trim().to_string();
        if name.is_empty() {
            skipped += 1;
            continue;
        }
        let dup: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM components WHERE library_id = ?1 AND LOWER(name) = LOWER(?2)",
                params![lib, name],
                |r| r.get(0),
            )
            .map_err(err)?;
        if dup > 0 {
            skipped += 1;
            continue;
        }
        conn.execute(
            "INSERT INTO components
                (library_id, name, category, package, manufacturer, quantity, location, datasheet_url, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                lib,
                name,
                c.category,
                c.package,
                c.manufacturer,
                c.quantity.max(0),
                c.location,
                c.datasheet_url,
                c.notes,
            ],
        )
        .map_err(err)?;
        added += 1;
    }
    print_json(&serde_json::json!({ "added": added, "skipped": skipped }));
    Ok(())
}

fn cmd_datasheets(p: &Parsed) -> Result<(), String> {
    if !p.positional.is_empty() {
        return Err("用法：cmcli datasheets".into());
    }
    let dir = datasheet_dir()?;
    let conn = open_db()?;
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).map_err(err)?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().ok();
        let refs: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT l.name, c.name FROM components c
                     JOIN libraries l ON l.id = c.library_id
                     WHERE c.datasheet_url = ?1",
                )
                .map_err(err)?;
            let v = stmt
                .query_map(params![name.clone()], |r| {
                    Ok(format!("{} / {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            v
        };
        out.push(serde_json::json!({
            "file_name": name,
            "size_bytes": meta.as_ref().map(|m| m.len()).unwrap_or(0),
            "refs": refs,
        }));
    }
    print_json(&out);
    Ok(())
}

fn cmd_ds_delete(p: &Parsed) -> Result<(), String> {
    let file_name = need_str(p, 0, "手册文件名")?;
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err("只接受 datasheets 目录内的裸文件名".into());
    }
    let target = datasheet_dir()?.join(&file_name);
    if !target.is_file() {
        return Err(format!("文件不存在：{file_name}"));
    }
    let conn = open_db()?;
    let cleared = conn
        .execute(
            "UPDATE components SET datasheet_url = NULL WHERE datasheet_url = ?1",
            params![file_name],
        )
        .map_err(err)?;
    fs::remove_file(&target).map_err(err)?;
    print_json(&serde_json::json!({ "deleted_file": file_name, "cleared_refs": cleared }));
    Ok(())
}

// ---------------- 入口 ----------------

fn help_text() -> String {
    format!(
        r#"cmcli —— 元件库存管理器命令行工具（与图形界面共用同一数据库）

数据库位置：%APPDATA%\com.componentmanager.app\components.db（可用环境变量 CM_DB_PATH 覆盖）
输出：成功时 stdout 为 JSON；失败时 stderr 输出错误信息且退出码为 1。

用法：
  cmcli libs                                  列出全部元件库（id / 名称 / 元件数）
  cmcli lib-create <库名>                      新建元件库
  cmcli list <库id>                            列出库内全部元件
  cmcli find <库id> <关键词>                    按名称/类别/封装/位置模糊搜索
  cmcli add <库id> <名称> [选项]                新增元件
  cmcli update <库id> <元件id> [选项]           按 id 更新元件字段
  cmcli adjust <库id> <名称> <±数量>            库存增减（如 +20 / -3），按名称精确匹配
  cmcli delete <库id> <元件id>                  删除元件
  cmcli export <库id> <文件路径>                导出库为 JSON（与 GUI 导出文件格式一致）
  cmcli import <库id> <文件路径>                从 JSON 导入到库（同名元件跳过）
  cmcli datasheets                             列出本地手册文件及引用情况
  cmcli ds-delete <手册文件名>                  删除本地手册（同步清除引用）

add / update 选项（均可选）：
  --category <类别>   --package <封装>   --manufacturer <制造商>
  --qty <数量>        --location <位置>   --notes <备注>
  --datasheet <值>    http(s) 链接原样保存；本地 PDF 路径会复制进
                      %APPDATA%\com.componentmanager.app\datasheets\（同名复用）

示例（入库一批元件）：
  cmcli libs
  cmcli add 1 "STM32F103C8T6" --category IC --package LQFP48 --manufacturer ST --qty 5 --location B03-R1 --datasheet "D:\datasheet\stm32f1.pdf"
  cmcli adjust 1 "10k电阻" -3
  cmcli find 1 0805"#
    )
}

fn run() -> i32 {
    let p = match parse_args() {
        Ok(p) => p,
        Err(e) => {
            if e == "USAGE" {
                println!("{}", help_text());
                return 0;
            }
            eprintln!("错误：{e}");
            return 2;
        }
    };
    let r = match p.cmd.as_str() {
        "libs" => cmd_libs(&p),
        "lib-create" => cmd_lib_create(&p),
        "list" => cmd_list(&p),
        "find" => cmd_find(&p),
        "add" => cmd_add(&p),
        "update" => cmd_update(&p),
        "adjust" => cmd_adjust(&p),
        "delete" => cmd_delete(&p),
        "export" => cmd_export(&p),
        "import" => cmd_import(&p),
        "datasheets" => cmd_datasheets(&p),
        "ds-delete" => cmd_ds_delete(&p),
        other => Err(format!("未知命令：{other}（运行 cmcli help 查看全部命令）")),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("错误：{e}");
            1
        }
    }
}

fn main() {
    std::process::exit(run());
}
