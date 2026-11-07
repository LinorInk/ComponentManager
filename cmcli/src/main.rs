// cmcli —— 元件库存管理器命令行工具
// 数据层全部来自共享库 cm-core：表结构、迁移、查询、增删改、导入导出只保留一份实现。
// 本文件只做「参数 → core 调用 → JSON 输出」的薄封装。
// 用法见根目录《命令行使用说明.md》或运行 `cmcli help`。

use cm_core::models::{self, Component};
use cm_core::{appdir, components, datasheet, transfer};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

// ---------------- 参数解析 ----------------

/// 解析结果：子命令 + 位置参数 + --key value 选项
struct Parsed {
    cmd: String,
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

/// 解析 argv。无参数或 help 时返回 "USAGE" 由入口转成帮助文本。
fn parse_args() -> Result<Parsed, String> {
    let raw: Vec<String> = env::args().skip(1).collect();
    if raw.is_empty() || raw[0] == "help" || raw[0] == "--help" || raw[0] == "-h" {
        return Err("USAGE".into());
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
    s.parse::<i64>()
        .map_err(|_| format!("库 id 必须是数字，收到：{s}"))
}

fn need_comp_id(p: &Parsed) -> Result<i64, String> {
    let s = need_str(p, 1, "元件 id")?;
    s.parse::<i64>()
        .map_err(|_| format!("元件 id 必须是数字：{s}"))
}

fn need_int(flag: &str, flags: &HashMap<String, String>) -> Result<i64, String> {
    let v = flags.get(flag).ok_or_else(|| format!("缺少 --{flag}"))?;
    v.parse::<i64>()
        .map_err(|_| format!("--{flag} 必须是整数，收到：{v}"))
}

// ---------------- 帮助文本 ----------------

fn help_text() -> &'static str {
    r#"cmcli —— 元件库存管理器命令行工具

数据库位置：%APPDATA%\com.componentmanager.app\components.db
输出：成功时 stdout 为 JSON；失败时 stderr 输出错误信息且退出码为 1。

用法：
  cmcli libs                                  列出全部元件库
  cmcli lib-create <库名>                      新建元件库
  cmcli list <库id>                            列出库内全部元件
  cmcli find <库id> <关键词>                    按名称/类别/封装/位置模糊搜索
  cmcli add <库id> <名称> [选项]                新增元件
  cmcli update <库id> <元件id> [选项]           按 id 更新元件字段
  cmcli adjust <库id> <名称> <±数量>            库存增减，按名称精确匹配
  cmcli delete <库id> <元件id>                  删除元件
  cmcli export <库id> <文件路径>                导出库为 JSON
  cmcli import <库id> <文件路径>                从 JSON 导入到库
  cmcli datasheets                             列出本地手册文件及引用情况
  cmcli ds-delete <手册文件名>                  删除本地手册
  cmcli path-add                               把 cmcli 所在目录加入用户 PATH
  cmcli path-remove                            从用户 PATH 移除 cmcli 所在目录

add / update 选项：
  --category <类别>   --package <封装>   --manufacturer <制造商>
  --qty <数量>        --location <位置>   --notes <备注>
  --precision <精度>  电阻/电容/电感的精度
  --datasheet <值>    http(s) 链接原样保存；本地 PDF 路径会复制进
                      %APPDATA%\com.componentmanager.app\datasheets\

示例：
  cmcli libs
  cmcli add 1 "STM32F103C8T6" --category IC --package LQFP48 --manufacturer ST --qty 5 --location B03-R1 --datasheet "D:\datasheet\stm32f1.pdf"
  cmcli adjust 1 "10k电阻" -3
  cmcli find 1 0805"#
}

// ---------------- 用户级 PATH 管理（仅 Windows） ----------------

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// cmcli.exe 所在目录
fn self_dir() -> Result<PathBuf, String> {
    let exe = env::current_exe().map_err(err)?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法确定程序所在目录".to_string())
}

#[cfg(windows)]
fn user_path() -> Result<String, String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;
    let env_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ)
        .map_err(err)?;
    Ok(match env_key.get_raw_value("Path") {
        Ok(v) => {
            let s = String::from_utf16_lossy(utf16_bytes_to_u16(&v.bytes));
            s.trim_end_matches('\0').to_string()
        }
        Err(_) => String::new(),
    })
}

#[cfg(windows)]
fn utf16_bytes_to_u16(bytes: &[u8]) -> &[u16] {
    // 注册表 REG_*_SZ 为偶数字节 UTF-16LE，此转换安全
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2) }
}

#[cfg(windows)]
fn write_user_path(value: &str) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE, REG_EXPAND_SZ};
    use winreg::RegKey;
    let env_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_WRITE)
        .map_err(err)?;
    env_key
        .set_raw_value(
            "Path",
            &winreg::RegValue {
                bytes: value
                    .encode_utf16()
                    .flat_map(|c| c.to_ne_bytes())
                    .chain([0, 0, 0, 0])
                    .collect(),
                vtype: REG_EXPAND_SZ,
            },
        )
        .map_err(err)
}

/// 广播 WM_SETTINGCHANGE，让资源管理器刷新用户环境变量：
/// 新开的终端立即能用上新的 PATH，无需注销或重启 Explorer
#[cfg(windows)]
fn broadcast_env_change() {
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: *mut c_void,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const HWND_BROADCAST: usize = 0xffff;
    let target: Vec<u16> = "Environment".encode_utf16().chain([0u16]).collect();
    let mut res: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as *mut c_void,
            WM_SETTINGCHANGE,
            0,
            target.as_ptr(),
            SMTO_ABORTIFHUNG,
            5000,
            &mut res,
        );
    }
}

/// 把 cmcli 所在目录加入用户 PATH（幂等）。返回 (状态, 目录)。
#[cfg(windows)]
fn path_add() -> Result<(&'static str, String), String> {
    let dir_s = self_dir()?.to_string_lossy().to_string();
    let cur = user_path()?;
    if is_present(&cur, &dir_s) {
        return Ok(("already", dir_s));
    }
    let new_val = if cur.is_empty() {
        dir_s.clone()
    } else {
        format!("{};{}", cur.trim_end_matches(';'), dir_s)
    };
    write_user_path(&new_val)?;
    broadcast_env_change();
    Ok(("added", dir_s))
}

/// 从用户 PATH 移除 cmcli 所在目录，不动其他条目。返回 (状态, 目录)。
#[cfg(windows)]
fn path_remove() -> Result<(&'static str, String), String> {
    let dir_s = self_dir()?.to_string_lossy().to_string();
    let cur = user_path()?;
    if !is_present(&cur, &dir_s) {
        return Ok(("not-in-path", dir_s));
    }
    let kept: Vec<&str> = cur
        .split(';')
        .filter(|x| !x.trim_end_matches('\\').eq_ignore_ascii_case(&dir_s))
        .collect();
    write_user_path(&kept.join(";"))?;
    broadcast_env_change();
    Ok(("removed", dir_s))
}

/// PATH 条目比较：忽略大小写与结尾反斜杠
#[cfg(windows)]
fn is_present(cur: &str, dir: &str) -> bool {
    cur.split(';')
        .any(|x| x.trim_end_matches('\\').eq_ignore_ascii_case(dir))
}

#[cfg(not(windows))]
fn path_add() -> Result<(&'static str, String), String> {
    Err("仅 Windows 支持 PATH 管理".into())
}

#[cfg(not(windows))]
fn path_remove() -> Result<(&'static str, String), String> {
    Err("仅 Windows 支持 PATH 管理".into())
}

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

// ---------------- PATH 命令 ----------------

fn cmd_path_add(_p: &Parsed) -> Result<(), String> {
    let (status, dir) = path_add()?;
    let mut out = serde_json::json!({ "status": status, "path_dir": dir });
    if status == "added" {
        out["note"] = serde_json::json!("新开的终端即可直接运行 cmcli");
    }
    print_json(&out);
    Ok(())
}

fn cmd_path_remove(_p: &Parsed) -> Result<(), String> {
    let (status, dir) = path_remove()?;
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
            println!("{}", help_text());
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
