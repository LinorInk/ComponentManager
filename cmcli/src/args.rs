//! 命令行参数解析：cmcli 独有，不进共享库。

use std::collections::HashMap;
use std::env;

/// 解析结果：子命令 + 位置参数 + --key value 选项
pub struct Parsed {
    pub cmd: String,
    pub positional: Vec<String>,
    pub flags: HashMap<String, String>,
}

/// 解析 argv。无参数或 help 时返回 "USAGE" 由入口转成帮助文本。
pub fn parse_args() -> Result<Parsed, String> {
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

pub fn need_str(p: &Parsed, idx: usize, what: &str) -> Result<String, String> {
    p.positional
        .get(idx)
        .cloned()
        .ok_or_else(|| format!("缺少参数：{what}"))
}

pub fn need_id(p: &Parsed, idx: usize) -> Result<i64, String> {
    let s = need_str(p, idx, "库 id")?;
    s.parse::<i64>()
        .map_err(|_| format!("库 id 必须是数字，收到：{s}"))
}

pub fn need_comp_id(p: &Parsed) -> Result<i64, String> {
    let s = need_str(p, 1, "元件 id")?;
    s.parse::<i64>()
        .map_err(|_| format!("元件 id 必须是数字：{s}"))
}

pub fn need_int(flag: &str, flags: &HashMap<String, String>) -> Result<i64, String> {
    let v = flags.get(flag).ok_or_else(|| format!("缺少 --{flag}"))?;
    v.parse::<i64>()
        .map_err(|_| format!("--{flag} 必须是整数，收到：{v}"))
}
