#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::{Manager, State};

/// 多库结构：libraries 为库，components.library_id 归属某个库
/// 首次运行自动建表；数据落在用户数据目录（避免 Program Files 无写权限）
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
    precision TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_components_name ON components(name);
CREATE INDEX IF NOT EXISTS idx_components_category ON components(category);
"#;

/// 按库索引须在确认 library_id 列存在后再建（旧库可能缺该列）
const LIB_INDEX_SQL: &str = "CREATE INDEX IF NOT EXISTS idx_components_library ON components(library_id)";

const COLS: &str =
    "id, name, category, package, manufacturer, quantity, location, datasheet_url, notes, precision, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub quantity: i64,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub datasheet_url: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub precision: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

struct Db {
    conn: Mutex<Connection>,
    path: String,
    data_dir: PathBuf,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// 空白字符串按 NULL 存，前端显示为“—”
fn norm(v: &Option<String>) -> Option<String> {
    v.as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
        precision: row.get(9)?,
        created_at: row.get(10)?,
    })
}

/// 检查 components 表是否已有某列（用于老库补列迁移）
fn has_component_col(conn: &Connection, col: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(components)")?;
    let mut found = false;
    let mut rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    while let Some(r) = rows.next() {
        if r? == col {
            found = true;
        }
    }
    Ok(found)
}

/// 建表并兼容旧版数据：补 library_id、precision 列，旧记录归入“默认库”
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;

    if !has_component_col(conn, "library_id")? {
        conn.execute(
            "ALTER TABLE components ADD COLUMN library_id INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    if !has_component_col(conn, "precision")? {
        conn.execute("ALTER TABLE components ADD COLUMN precision TEXT", [])?;
    }

    let lib_cnt: i64 = conn.query_row("SELECT COUNT(*) FROM libraries", [], |r| r.get(0))?;
    if lib_cnt == 0 {
        conn.execute(
            "INSERT INTO libraries (id, name) VALUES (1, '默认库')",
            [],
        )?;
    }
    conn.execute(
        "UPDATE components SET library_id = 1 WHERE library_id IS NULL OR library_id = 0",
        [],
    )?;
    conn.execute(LIB_INDEX_SQL, [])?;
    Ok(())
}

fn lib_name_taken(conn: &Connection, name: &str, except_id: i64) -> Result<bool, String> {
    let cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM libraries WHERE name = ?1 AND id <> ?2",
            params![name, except_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(cnt > 0)
}

fn open_db(app: &tauri::App) -> Result<(Connection, String, PathBuf), Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    fs::create_dir_all(&dir)?;
    let file = dir.join("components.db");
    let path = file.to_string_lossy().to_string();
    let conn = Connection::open(&file)?;
    migrate(&conn)?;
    Ok((conn, path, dir))
}

#[tauri::command]
fn db_path(db: State<'_, Db>) -> String {
    db.path.clone()
}

/// 手册文件存放目录：数据目录下的 datasheets 文件夹
fn datasheet_dir(db: &Db) -> Result<PathBuf, String> {
    let dir = db.data_dir.join("datasheets");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 导入结果：stored 文件名 + 是否因同名而复用已有文件
#[derive(Debug, Serialize)]
struct DatasheetImport {
    file_name: String,
    reused: bool,
}

/// 把选中的本地手册复制进 datasheets 目录。
/// 同名文件跳过复制、直接复用已有的那份（多个元件可共享同一手册），由前端弹窗提醒。
fn import_datasheet(db: &Db, src: &str) -> Result<DatasheetImport, String> {
    let src_path = PathBuf::from(src);
    if !src_path.is_file() {
        return Err("所选文件不存在".into());
    }
    let dir = datasheet_dir(db)?;
    // 来自磁盘的真实文件名，天然不含路径分隔符
    let file_name = src_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "datasheet.pdf".into());
    let target = dir.join(&file_name);
    if target.exists() {
        return Ok(DatasheetImport { file_name, reused: true });
    }
    fs::copy(src_path.as_path(), &target).map_err(err)?;
    Ok(DatasheetImport { file_name, reused: false })
}

/// 弹出系统文件选择框，选中后复制进 datasheets 目录；取消返回 None
#[tauri::command]
fn pick_datasheet(
    app: tauri::AppHandle,
    db: State<'_, Db>,
) -> Result<Option<DatasheetImport>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("选择数据手册文件（PDF）")
        .add_filter("PDF", &["pdf"])
        .blocking_pick_file();
    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp
        .into_path()
        .map_err(|e| format!("无法读取所选文件路径：{e}"))?;
    let stored = import_datasheet(&db, &path.to_string_lossy())?;
    Ok(Some(stored))
}

/// 打开已存放的手册文件（交给系统关联程序，PDF 通常是浏览器新标签）
#[tauri::command]
fn open_datasheet(db: State<'_, Db>, file_name: String) -> Result<bool, String> {
    // 只允许 datasheets 目录内的裸文件名，防目录穿越
    if file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || file_name.is_empty()
    {
        return Err("非法的手册文件名".into());
    }
    let target = datasheet_dir(&db)?.join(&file_name);
    if !target.is_file() {
        return Err(format!("手册文件不存在：{file_name}（可能在文件夹中被移动或删除）"));
    }
    tauri_plugin_opener::open_path(&target, None::<&str>).map_err(err)?;
    Ok(true)
}

#[derive(Debug, Serialize)]
struct DatasheetInfo {
    file_name: String,
    size: u64,
    mtime: String,
    refs: Vec<String>, // 引用该文件的“库名 / 元件名”，空 = 无引用可安全删
}

/// 扫描 datasheets 目录，附带每个文件被哪些库的哪些元件引用
#[tauri::command]
fn list_datasheets(db: State<'_, Db>) -> Result<Vec<DatasheetInfo>, String> {
    let dir = datasheet_dir(&db)?;
    let entries = fs::read_dir(&dir).map_err(err)?;
    let conn = db.conn.lock().map_err(err)?;

    let mut out: Vec<DatasheetInfo> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        // 引用查询：datasheet_url 精确等于该文件名
        let mut stmt = conn
            .prepare(
                "SELECT l.name, c.name FROM components c
                 JOIN libraries l ON l.id = c.library_id
                 WHERE c.datasheet_url = ?1
                 ORDER BY l.name, c.name",
            )
            .map_err(err)?;
        let refs = stmt
            .query_map(params![file_name.clone()], |row| {
                let lib: String = row.get(0)?;
                let comp: String = row.get(1)?;
                Ok(format!("{lib} / {comp}"))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;

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

fn mtime_of(path: &std::path::Path) -> String {
    let Ok(t) = fs::metadata(path).and_then(|m| m.modified()) else {
        return String::new();
    };
    // 不引额外时间库：手动把 UNIX 时间换算为本地大致时间（UTC+8 固定偏移，展示用足够）
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

/// 删除手册文件；若仍被引用，同步清除引用元件的 datasheet_url
#[tauri::command]
fn delete_datasheet(db: State<'_, Db>, file_name: String) -> Result<i64, String> {
    if file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || file_name.is_empty()
    {
        return Err("非法的手册文件名".into());
    }
    let target = datasheet_dir(&db)?.join(&file_name);
    if !target.is_file() {
        return Err(format!("文件不存在：{file_name}"));
    }
    let conn = db.conn.lock().map_err(err)?;
    let cleared = conn
        .execute(
            "UPDATE components SET datasheet_url = NULL WHERE datasheet_url = ?1",
            params![file_name],
        )
        .map_err(err)?;
    fs::remove_file(&target).map_err(err)?;
    Ok(cleared as i64)
}

/* ---------------- 元件库 ---------------- */

#[tauri::command]
fn list_libraries(db: State<'_, Db>) -> Result<Vec<Library>, String> {
    let conn = db.conn.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.name,
                    (SELECT COUNT(*) FROM components c WHERE c.library_id = l.id)
             FROM libraries l ORDER BY l.id ASC",
        )
        .map_err(err)?;
    let out = stmt
        .query_map([], |row| {
            Ok(Library {
                id: row.get(0)?,
                name: row.get(1)?,
                count: row.get(2)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(out)
}

#[tauri::command]
fn create_library(db: State<'_, Db>, name: String) -> Result<i64, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("库名不能为空".into());
    }
    let conn = db.conn.lock().map_err(err)?;
    if lib_name_taken(&conn, &name, 0)? {
        return Err(format!("库名「{name}」已存在，请换一个"));
    }
    conn.execute(
        "INSERT INTO libraries (name) VALUES (?1)",
        params![name],
    )
    .map_err(err)?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
fn rename_library(db: State<'_, Db>, id: i64, name: String) -> Result<bool, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("库名不能为空".into());
    }
    let conn = db.conn.lock().map_err(err)?;
    if lib_name_taken(&conn, &name, id)? {
        return Err(format!("库名「{name}」已存在，请换一个"));
    }
    let changed = conn
        .execute("UPDATE libraries SET name = ?2 WHERE id = ?1", params![id, name])
        .map_err(err)?;
    Ok(changed > 0)
}

/// 删除库及其全部元件；至少保留一个库
#[tauri::command]
fn delete_library(db: State<'_, Db>, id: i64) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(err)?;
    let libs: i64 = conn
        .query_row("SELECT COUNT(*) FROM libraries", [], |r| r.get(0))
        .map_err(err)?;
    if libs <= 1 {
        return Err("至少需要保留一个元件库".into());
    }
    conn.execute("DELETE FROM components WHERE library_id = ?1", params![id])
        .map_err(err)?;
    let changed = conn
        .execute("DELETE FROM libraries WHERE id = ?1", params![id])
        .map_err(err)?;
    Ok(changed > 0)
}

/* ---------------- 库导入导出 ---------------- */

/// 分享文件格式：JSON，元件数据随文件走（手册 PDF 为本地文件，不随文件分享）
#[derive(Debug, Serialize, Deserialize)]
struct LibraryFile {
    app: String,
    version: u32,
    library: String,
    exported_at: String,
    components: Vec<Component>,
}

#[derive(Debug, Serialize)]
struct ImportResult {
    added: i64,
    skipped: i64,
}

const FILE_APP: &str = "ComponentManager";

/// 把当前库全部元件导出为 JSON 文件（弹出保存对话框）；取消返回 None
#[tauri::command]
fn export_library(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    library_id: i64,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (lib_name, comps, now) = {
        let conn = db.conn.lock().map_err(err)?;
        let lib_name: String = conn
            .query_row(
                "SELECT name FROM libraries WHERE id = ?1",
                params![library_id],
                |r| r.get(0),
            )
            .map_err(|_| "元件库不存在".to_string())?;
        let sql =
            format!("SELECT {COLS} FROM components WHERE library_id = ?1 ORDER BY name ASC");
        let mut stmt = conn.prepare(&sql).map_err(err)?;
        let comps = stmt
            .query_map(params![library_id], row_to_component)
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        let now: String = conn
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(err)?;
        (lib_name, comps, now)
    };

    let file = LibraryFile {
        app: FILE_APP.to_string(),
        version: 1,
        library: lib_name.clone(),
        exported_at: now,
        components: comps,
    };

    let default_name = format!("{lib_name}.json");
    let picked = app
        .dialog()
        .file()
        .set_title("导出元件库")
        .set_file_name(&default_name)
        .add_filter("JSON", &["json"])
        .blocking_save_file();
    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp
        .into_path()
        .map_err(|e| format!("无法确定保存路径：{e}"))?;
    let json = serde_json::to_string_pretty(&file).map_err(err)?;
    fs::write(&path, json).map_err(err)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

/// 从 JSON 文件导入元件到当前库；与库内重名（忽略大小写）的条目跳过
#[tauri::command]
fn import_library(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    library_id: i64,
) -> Result<Option<ImportResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app
        .dialog()
        .file()
        .set_title("选择要导入的元件库文件")
        .add_filter("JSON", &["json"])
        .blocking_pick_file();
    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp
        .into_path()
        .map_err(|e| format!("无法读取所选文件：{e}"))?;
    let text = fs::read_to_string(&path).map_err(err)?;
    let file: LibraryFile = serde_json::from_str(&text)
        .map_err(|e| format!("文件解析失败，不是有效的导出文件：{e}"))?;
    if file.app != FILE_APP {
        return Err("该文件不是元件库存管理器导出的文件".into());
    }

    let conn = db.conn.lock().map_err(err)?;
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM libraries WHERE id = ?1",
            params![library_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if exists == 0 {
        return Err("元件库不存在，请先选择有效的库".into());
    }

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
    Ok(Some(ImportResult { added, skipped }))
}

/* ---------------- 元件 ---------------- */

#[tauri::command]
fn list_all_components(db: State<'_, Db>, library_id: i64) -> Result<Vec<Component>, String> {
    let conn = db.conn.lock().map_err(err)?;
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
#[tauri::command]
fn search_components(
    db: State<'_, Db>,
    library_id: i64,
    keyword: String,
) -> Result<Vec<Component>, String> {
    let kw = keyword.trim().to_string();
    if kw.is_empty() {
        return list_all_components(db, library_id);
    }
    let conn = db.conn.lock().map_err(err)?;
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

#[tauri::command]
fn add_component(db: State<'_, Db>, library_id: i64, comp: Component) -> Result<i64, String> {
    let name = comp.name.trim().to_string();
    if name.is_empty() {
        return Err("元件名称不能为空".into());
    }
    let conn = db.conn.lock().map_err(err)?;
    let exists: i64 = conn
        .query_row("SELECT COUNT(*) FROM libraries WHERE id = ?1", params![library_id], |r| r.get(0))
        .map_err(err)?;
    if exists == 0 {
        return Err("元件库不存在，请先选择有效的库".into());
    }
    conn.execute(
        "INSERT INTO components
            (library_id, name, category, package, manufacturer, quantity, location, datasheet_url, notes, precision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            library_id,
            name,
            norm(&comp.category),
            norm(&comp.package),
            norm(&comp.manufacturer),
            comp.quantity.max(0),
            norm(&comp.location),
            norm(&comp.datasheet_url),
            norm(&comp.notes),
            norm(&comp.precision),
        ],
    )
    .map_err(err)?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
fn update_component(db: State<'_, Db>, comp: Component) -> Result<bool, String> {
    let name = comp.name.trim().to_string();
    if name.is_empty() {
        return Err("元件名称不能为空".into());
    }
    let conn = db.conn.lock().map_err(err)?;
    let changed = conn
        .execute(
            "UPDATE components SET
                name = ?2, category = ?3, package = ?4, manufacturer = ?5,
                quantity = ?6, location = ?7, datasheet_url = ?8, notes = ?9, precision = ?10
             WHERE id = ?1",
            params![
                comp.id,
                name,
                norm(&comp.category),
                norm(&comp.package),
                norm(&comp.manufacturer),
                comp.quantity.max(0),
                norm(&comp.location),
                norm(&comp.datasheet_url),
                norm(&comp.notes),
                norm(&comp.precision),
            ],
        )
        .map_err(err)?;
    Ok(changed > 0)
}

#[tauri::command]
fn delete_component(db: State<'_, Db>, id: i64) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(err)?;
    let changed = conn
        .execute("DELETE FROM components WHERE id = ?1", params![id])
        .map_err(err)?;
    Ok(changed > 0)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let (conn, path, data_dir) = open_db(app)?;
            app.manage(Db {
                conn: Mutex::new(conn),
                path,
                data_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db_path,
            pick_datasheet,
            open_datasheet,
            list_datasheets,
            delete_datasheet,
            list_libraries,
            create_library,
            rename_library,
            delete_library,
            export_library,
            import_library,
            list_all_components,
            search_components,
            add_component,
            update_component,
            delete_component
        ])
        .run(tauri::generate_context!())
        .expect("元件库存管理器启动失败");
}
