//! 数据模型与表结构：GUI 与 CLI 共用的唯一一份定义。
//! 新增字段时只需改这里的 SCHEMA / COLS / Component / 迁移，两边自动一致。

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

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

pub const COLS: &str =
    "id, name, category, package, manufacturer, quantity, location, datasheet_url, notes, precision, created_at";

/// 分享文件格式的应用标识
pub const FILE_APP: &str = "ComponentManager";

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

impl Default for Component {
    /// 空白元件：调用方按需逐字段填充，未填的字段写库时按 NULL / 0 处理
    fn default() -> Self {
        Component {
            id: 0,
            name: String::new(),
            category: None,
            package: None,
            manufacturer: None,
            quantity: 0,
            location: None,
            datasheet_url: None,
            notes: None,
            precision: None,
            created_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

/// 导入结果：stored 文件名 + 是否因同名而复用已有文件
#[derive(Debug, Serialize)]
pub struct DatasheetImport {
    pub file_name: String,
    pub reused: bool,
}

/// 本地手册文件信息：大小、修改时间、被哪些库的哪些元件引用
#[derive(Debug, Serialize)]
pub struct DatasheetInfo {
    pub file_name: String,
    pub size: u64,
    pub mtime: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub added: i64,
    pub skipped: i64,
}

/// 分享文件格式：JSON，元件数据随文件走（手册 PDF 为本地文件，不随文件分享）
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryFile {
    pub app: String,
    pub version: u32,
    pub library: String,
    pub exported_at: String,
    pub components: Vec<Component>,
}

pub fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// 空白字符串按 NULL 存，前端显示为“—”
pub fn norm(v: &Option<String>) -> Option<String> {
    v.as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// CLI 的裸字符串入参同样语义：trim 后空串视为 None
pub fn norm_str(v: &str) -> Option<String> {
    let t = v.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

pub fn row_to_component(row: &Row) -> rusqlite::Result<Component> {
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

/// components 表是否已有某列（用于老库补列迁移）
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
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
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
        conn.execute("INSERT INTO libraries (id, name) VALUES (1, '默认库')", [],)?;
    }
    conn.execute(
        "UPDATE components SET library_id = 1 WHERE library_id IS NULL OR library_id = 0",
        [],
    )?;
    conn.execute(LIB_INDEX_SQL, [])?;
    Ok(())
}

pub fn lib_name_taken(conn: &Connection, name: &str, except_id: i64) -> Result<bool, String> {
    let cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM libraries WHERE name = ?1 AND id <> ?2",
            params![name, except_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(cnt > 0)
}

/// 校验库存在并返回库名
pub fn require_library(conn: &Connection, id: i64) -> Result<String, String> {
    conn.query_row(
        "SELECT name FROM libraries WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )
    .map_err(|_| format!("元件库不存在：id={id}"))
}

/// 新建元件库：库名去空白后不可为空，且不能与已有库重名。
/// 返回新库 id 与落库后的库名。
pub fn create_library(conn: &Connection, name: &str) -> Result<(i64, String), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("库名不能为空".into());
    }
    if lib_name_taken(conn, &name, 0)? {
        return Err(format!("库名「{name}」已存在"));
    }
    conn.execute("INSERT INTO libraries (name) VALUES (?1)", params![name])
        .map_err(err)?;
    Ok((conn.last_insert_rowid(), name))
}

/// 列出全部库及各自的元件数量
pub fn list_libraries(conn: &Connection) -> Result<Vec<Library>, String> {
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
