//! 元件库存管理器 —— 图形界面入口（Tauri 后端）
//! 存储层逻辑全部来自共享库 cm-core，本文件只保留：
//! Tauri 命令壳、系统对话框、用默认程序打开文件、应用装配。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cm_core::models::{self, Component, DatasheetImport, DatasheetInfo, ImportResult, Library};
use cm_core::{components, datasheet, db as cmdb, transfer};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

/// GUI 侧数据库状态：连接 + 数据目录
struct Db {
    conn: Mutex<Connection>,
    path: String,
    data_dir: PathBuf,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn with_conn<T, F>(db: &Db, f: F) -> Result<T, String>
where
    F: FnOnce(&Connection) -> Result<T, String>,
{
    let conn = db.conn.lock().map_err(err)?;
    f(&conn)
}

fn open_db(app: &tauri::App) -> Result<(Connection, String, PathBuf), Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("components.db");
    let path = file.to_string_lossy().to_string();
    let conn = cmdb::open_at(&file)?;
    Ok((conn, path, dir))
}

#[tauri::command]
fn db_path(db: State<'_, Db>) -> String {
    db.path.clone()
}

/* ---------------- 数据手册 ---------------- */

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
        .set_title("选择数据手册文件")
        .add_filter("PDF", &["pdf"])
        .blocking_pick_file();
    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp
        .into_path()
        .map_err(|e| format!("无法读取所选文件路径：{e}"))?;
    Ok(Some(datasheet::import_file(&db.data_dir, &path.to_string_lossy())?))
}

/// 打开已存放的手册文件（交给系统关联程序，PDF 通常是浏览器新标签）
#[tauri::command]
fn open_datasheet(db: State<'_, Db>, file_name: String) -> Result<bool, String> {
    let target = datasheet::resolve(&db.data_dir, &file_name)?;
    tauri_plugin_opener::open_path(&target, None::<&str>).map_err(err)?;
    Ok(true)
}

/// 列出本地手册文件及引用情况
#[tauri::command]
fn list_datasheets(db: State<'_, Db>) -> Result<Vec<DatasheetInfo>, String> {
    with_conn(&db, |conn| datasheet::list_with_refs(&db.data_dir, conn))
}

/// 删除手册文件；同步清除引用它的元件上的手册标记
#[tauri::command]
fn delete_datasheet(db: State<'_, Db>, file_name: String) -> Result<i64, String> {
    with_conn(&db, |conn| datasheet::delete(&db.data_dir, conn, &file_name))
}

/* ---------------- 元件库 ---------------- */

#[tauri::command]
fn list_libraries(db: State<'_, Db>) -> Result<Vec<Library>, String> {
    with_conn(&db, |conn| models::list_libraries(conn))
}

#[tauri::command]
fn create_library(db: State<'_, Db>, name: String) -> Result<i64, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("库名不能为空".into());
    }
    with_conn(&db, |conn| {
        if models::lib_name_taken(conn, &name, 0)? {
            return Err(format!("库名「{name}」已存在，请换一个"));
        }
        conn.execute("INSERT INTO libraries (name) VALUES (?1)", rusqlite::params![name])
            .map_err(err)?;
        Ok(conn.last_insert_rowid())
    })
}

#[tauri::command]
fn rename_library(db: State<'_, Db>, id: i64, name: String) -> Result<bool, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("库名不能为空".into());
    }
    with_conn(&db, |conn| {
        if models::lib_name_taken(conn, &name, id)? {
            return Err(format!("库名「{name}」已存在，请换一个"));
        }
        let changed = conn
            .execute(
                "UPDATE libraries SET name = ?2 WHERE id = ?1",
                rusqlite::params![id, name],
            )
            .map_err(err)?;
        Ok(changed > 0)
    })
}

/// 删除库及其全部元件；至少保留一个库
#[tauri::command]
fn delete_library(db: State<'_, Db>, id: i64) -> Result<bool, String> {
    with_conn(&db, |conn| {
        let libs: i64 = conn
            .query_row("SELECT COUNT(*) FROM libraries", [], |r| r.get(0))
            .map_err(err)?;
        if libs <= 1 {
            return Err("至少需要保留一个元件库".into());
        }
        conn.execute(
            "DELETE FROM components WHERE library_id = ?1",
            rusqlite::params![id],
        )
        .map_err(err)?;
        let changed = conn
            .execute("DELETE FROM libraries WHERE id = ?1", rusqlite::params![id])
            .map_err(err)?;
        Ok(changed > 0)
    })
}

/* ---------------- 库导入导出 ---------------- */

/// 把当前库全部元件导出为 JSON 文件（弹出保存对话框）；取消返回 None
#[tauri::command]
fn export_library(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    library_id: i64,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let file = with_conn(&db, |conn| transfer::build_file(conn, library_id))?;

    let picked = app
        .dialog()
        .file()
        .set_title("导出元件库")
        .set_file_name(format!("{}.json", file.library))
        .add_filter("JSON", &["json"])
        .blocking_save_file();
    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp
        .into_path()
        .map_err(|e| format!("无法确定保存路径：{e}"))?;
    std::fs::write(&path, transfer::to_pretty(&file)?).map_err(err)?;
    Ok(Some(path.to_string_lossy().to_string()))
}

/// 从 JSON 文件导入元件到当前库；与库内重名的条目跳过
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
    let text = std::fs::read_to_string(&path).map_err(err)?;
    let file = transfer::parse_file(&text)?;

    with_conn(&db, |conn| {
        models::require_library(conn, library_id)?;
        transfer::merge_into(conn, library_id, &file)
    })
    .map(Some)
}

/* ---------------- 元件 ---------------- */

#[tauri::command]
fn list_all_components(db: State<'_, Db>, library_id: i64) -> Result<Vec<Component>, String> {
    with_conn(&db, |conn| components::list_all(conn, library_id))
}

#[tauri::command]
fn search_components(
    db: State<'_, Db>,
    library_id: i64,
    keyword: String,
) -> Result<Vec<Component>, String> {
    with_conn(&db, |conn| components::search(conn, library_id, &keyword))
}

#[tauri::command]
fn add_component(db: State<'_, Db>, library_id: i64, comp: Component) -> Result<i64, String> {
    with_conn(&db, |conn| {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM libraries WHERE id = ?1",
                rusqlite::params![library_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if exists == 0 {
            return Err("元件库不存在，请先选择有效的库".into());
        }
        components::insert(conn, library_id, &comp)
    })
}

#[tauri::command]
fn update_component(db: State<'_, Db>, comp: Component) -> Result<bool, String> {
    with_conn(&db, |conn| components::update_full(conn, &comp))
}

#[tauri::command]
fn delete_component(db: State<'_, Db>, id: i64) -> Result<bool, String> {
    with_conn(&db, |conn| components::delete_by_id(conn, id))
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
