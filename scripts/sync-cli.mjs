// tauri build 前置脚本：编译独立 crate cmcli 的发行版，并同步为安装包的外部程序
// （tauri 要求 externalBin 的实体文件命名为 <名称>-<目标三元组>.exe，打包时会自动改回 cmcli.exe 放在主程序旁）
// cmcli 单独成 crate：产物不在 src-tauri/target 里，安装包不会同时收集到两份 cmcli.exe（避免 WiX ICE30 冲突）
import { execSync } from "node:child_process";
import { cpSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const TRIPLE = "x86_64-pc-windows-msvc";

// 定位 cargo：PATH 优先，否则用 rustup 默认安装位置
const cargoCandidates = ["cargo", join(homedir(), ".cargo", "bin", "cargo.exe")];
let cargoCmd = null;
for (const c of cargoCandidates) {
  const probe = c === "cargo" ? "cargo" : `"${c}"`;
  try {
    execSync(`${probe} --version`, { stdio: "ignore" });
    cargoCmd = probe;
    break;
  } catch {}
}
if (!cargoCmd) {
  console.error("找不到 cargo，请确认已安装 Rust 工具链");
  process.exit(1);
}

const relBuilt = join(root, "cmcli", "target", "release", "cmcli.exe");
const dbgBuilt = join(root, "cmcli", "target", "debug", "cmcli.exe");
const binDir = join(root, "src-tauri", "bin");
const dest = join(binDir, `cmcli-${TRIPLE}.exe`);

mkdirSync(binDir, { recursive: true });

// 独立 crate：build.rs 校验 externalBin 实体存在早于编译，故需先占位
if (!existsSync(dest)) {
  if (existsSync(relBuilt)) {
    cpSync(relBuilt, dest);
  } else if (existsSync(dbgBuilt)) {
    cpSync(dbgBuilt, dest);
  } else {
    execSync(`${cargoCmd} build --bin cmcli --manifest-path cmcli/Cargo.toml`, {
      cwd: root,
      stdio: "inherit",
    });
    cpSync(dbgBuilt, dest);
  }
}

execSync(`${cargoCmd} build --release --bin cmcli --manifest-path cmcli/Cargo.toml`, {
  cwd: root,
  stdio: "inherit",
});
cpSync(relBuilt, dest); // 用 release 产物覆盖占位
console.log(`cmcli 已同步到安装包外部程序目录：${dest}`);
