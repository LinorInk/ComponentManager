// cmcli —— 元件库存管理器命令行工具
// 数据层全部来自共享库 cm-core：表结构、迁移、查询、增删改、导入导出只保留一份实现。
// 本文件只做「参数 → core 调用 → JSON 输出」的薄封装：
// 参数解析见 args.rs，帮助文本见 help.rs，用户 PATH 管理见 pathenv.rs。
// 用法见根目录《命令行使用说明.md》或运行 `cmcli help`。

mod args;
mod help;
mod pathenv;

use args::{need_comp_id, need_id, need_int, need_str, parse_args, Parsed};
use cm_core::models::{self, Component};
use cm_core::{appdir, components, datasheet, transfer};
use serde::Serialize;
use std::fs;

/// add / update 共用的文本选项（与 Component 字段同名）
const TEXT_OPTIONS: [&str; 6] = [
    "category",
    "package",
    "manufacturer",
    "location",
    "notes",
    "precision",
];

// ---------------- 输出 ----------------

/// 成功结果统一打印为格式化 JSON
fn print_json<T: Serialize>(v: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).expect("JSON 序列化失败")
    );
}

/// --datasheet 选项语义：链接原样存；本地文件复制进手册库后存裸文件名
fn datasheet_value(p: &Parsed) -> Result<Option<String>, String> {
    match p.flags.get("datasheet") {
        Some(v) => Ok(Some(datasheet::store_value(&appdir::data_dir()?, v)?)),
        None => Ok(None),
    }
}

/// 不接受位置参数的命令（libs / datasheets 等）做一次校验
fn no_args(p: &Parsed, usage: &str) -> Result<(), String> {
    if p.positional.is_empty() {
        Ok(())
    } else {
        Err(format!("用法：{usage}"))
    }
}

// ---------------- 元件库 ----------------

fn cmd_libs(p: &Parsed) -> Result<(), String> {
    no_args(p, "cmcli libs")?;
    print_json(&models::list_libraries(&appdir::open_db()?)?);
    Ok(())
}

fn cmd_lib_create(p: &Parsed) -> Result<(), String> {
    let name = need_str(p, 0, "库名")?;
    let conn = appdir::open_db()?;
    let (id, name) = models::create_library(&conn, &name)?;
    print_json(&serde_json::json!({ "id": id, "name": name }));
    Ok(())
}

// ---------------- 元件 ----------------

fn cmd_list(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    print_json(&components::list_all(&conn, lib)?);
    Ok(())
}

fn cmd_find(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let kw = need_str(p, 1, "关键词")?;
    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    print_json(&components::search(&conn, lib, &kw)?);
    Ok(())
}

fn cmd_add(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let opt = |key: &str| p.flags.get(key).and_then(|v| models::norm_str(v));
    let comp = Component {
        name: need_str(p, 1, "元件名称")?, // 名称合法性与空白处理交给 core
        category: opt("category"),
        package: opt("package"),
        manufacturer: opt("manufacturer"),
        location: opt("location"),
        notes: opt("notes"),
        precision: opt("precision"),
        quantity: if p.flags.contains_key("qty") {
            need_int("qty", &p.flags)?.max(0)
        } else {
            0
        },
        datasheet_url: datasheet_value(p)?,
        ..Component::default()
    };

    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    let id = components::insert(&conn, lib, &comp)?;
    print_json(&components::get_by_id(&conn, id)?);
    Ok(())
}

fn cmd_update(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let comp_id = need_comp_id(p)?;

    let mut patch = components::ComponentPatch::default();
    for key in TEXT_OPTIONS {
        if let Some(v) = p.flags.get(key) {
            if let Some(slot) = patch.text_mut(key) {
                *slot = models::norm_str(v);
            }
        }
    }
    if let Some(v) = p.flags.get("name") {
        patch.name = Some(models::norm_str(v).ok_or("名称不能为空")?);
    }
    if p.flags.contains_key("qty") {
        patch.quantity = Some(need_int("qty", &p.flags)?.max(0));
    }
    if p.flags.contains_key("datasheet") {
        patch.datasheet_url = Some(datasheet_value(p)?);
    }

    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    print_json(&components::apply_patch(&conn, lib, comp_id, &patch)?);
    Ok(())
}

fn cmd_adjust(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let name = need_str(p, 1, "元件名称")?;
    let delta: i64 = need_str(p, 2, "增减数量")?
        .parse()
        .map_err(|s| format!("数量必须是整数，收到：{s}"))?;

    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    print_json(&components::adjust_by_name(&conn, lib, &name, delta)?);
    Ok(())
}

fn cmd_delete(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let comp_id = need_comp_id(p)?;
    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    if !components::delete_in_library(&conn, comp_id, lib)? {
        return Err(format!("未找到元件 id={comp_id}，库 {lib}"));
    }
    print_json(&serde_json::json!({ "deleted": comp_id }));
    Ok(())
}

// ---------------- 库导入导出 ----------------

fn cmd_export(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let out = need_str(p, 1, "输出文件路径")?;
    let conn = appdir::open_db()?;
    let file = transfer::build_file(&conn, lib)?;
    let count = file.components.len();
    fs::write(&out, transfer::to_pretty(&file)?).map_err(|e| e.to_string())?;
    print_json(&serde_json::json!({ "exported": out, "count": count }));
    Ok(())
}

fn cmd_import(p: &Parsed) -> Result<(), String> {
    let lib = need_id(p, 0)?;
    let path = need_str(p, 1, "导入文件路径")?;
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file = transfer::parse_file(&text)?;
    let conn = appdir::open_db()?;
    models::require_library(&conn, lib)?;
    print_json(&transfer::merge_into(&conn, lib, &file)?);
    Ok(())
}

// ---------------- 本地手册管理 ----------------

fn cmd_datasheets(p: &Parsed) -> Result<(), String> {
    no_args(p, "cmcli datasheets")?;
    let conn = appdir::open_db()?;
    print_json(&datasheet::list_with_refs(&appdir::data_dir()?, &conn)?);
    Ok(())
}

fn cmd_ds_delete(p: &Parsed) -> Result<(), String> {
    let file_name = need_str(p, 0, "手册文件名")?;
    let conn = appdir::open_db()?;
    let cleared = datasheet::delete(&appdir::data_dir()?, &conn, &file_name)?;
    print_json(&serde_json::json!({ "deleted_file": file_name, "cleared_refs": cleared }));
    Ok(())
}

// ---------------- PATH 管理 ----------------

fn cmd_path_add(_p: &Parsed) -> Result<(), String> {
    let (status, dir) = pathenv::add()?;
    let mut out = serde_json::json!({ "status": status, "path_dir": dir });
    if status == "added" {
        out["note"] = serde_json::json!("新开的终端即可直接运行 cmcli");
    }
    print_json(&out);
    Ok(())
}

fn cmd_path_remove(_p: &Parsed) -> Result<(), String> {
    let (status, dir) = pathenv::remove()?;
    print_json(&serde_json::json!({ "status": status, "path_dir": dir }));
    Ok(())
}

// ---------------- 入口 ----------------

/// 命令表：子命令名 → 处理函数
const COMMANDS: &[(&str, fn(&Parsed) -> Result<(), String>)] = &[
    ("libs", cmd_libs),
    ("lib-create", cmd_lib_create),
    ("list", cmd_list),
    ("find", cmd_find),
    ("add", cmd_add),
    ("update", cmd_update),
    ("adjust", cmd_adjust),
    ("delete", cmd_delete),
    ("export", cmd_export),
    ("import", cmd_import),
    ("datasheets", cmd_datasheets),
    ("ds-delete", cmd_ds_delete),
    ("path-add", cmd_path_add),
    ("path-remove", cmd_path_remove),
];

fn dispatch(p: &Parsed) -> Result<(), String> {
    match COMMANDS.iter().find(|(name, _)| *name == p.cmd) {
        Some((_, handler)) => handler(p),
        None => Err(format!("未知命令：{}，运行 cmcli help 查看全部命令", p.cmd)),
    }
}

fn run() -> i32 {
    let p = match parse_args() {
        Ok(p) => p,
        Err(e) if e == "USAGE" => {
            println!("{}", help::help_text());
            return 0;
        }
        Err(e) => {
            eprintln!("错误：{e}");
            return 2;
        }
    };
    match dispatch(&p) {
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
