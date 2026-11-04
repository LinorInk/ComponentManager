# 元件库存管理器（Component Manager）

基于 Tauri 2 + React + SQLite 的轻量级桌面应用，用于个人电子元器件的入库、查询和管理。

## 功能

- **多元件库**：可建立多个库区分不同人 / 项目的元件；启动时进入库选择页，双击卡片或点「进入」使用该库，顶栏可随时切换；支持新建、重命名、删除库（至少保留一个）。旧版单库数据启动时自动归入「默认库」
- **库导入导出（分享）**：状态栏「⬆ 导出本库」把当前库全部元件存为 JSON 文件，发给同事；对方在自己软件里「⬇ 导入到本库」选择该文件，重名元件自动跳过。注意：数据手册 PDF 是本地文件，不随 JSON 分享，需单独传
- **元件录入**：名称（必填）、类别、封装、制造商、数量、存放位置、数据手册链接、备注
- **模糊搜索**：在当前库内按名称 / 类别 / 封装 / 位置关键词搜索，回车即查
- **列表管理**：点击表头排序；低库存（≤5）高亮提醒；存放位置高亮显示
- **编辑与删除**：行内按钮，删除有二次确认
- **数据手册（本地 + 在线双模式）**：录入时可选本地 PDF（自动复制到数据目录 `datasheets` 文件夹）或粘贴在线链接，二者共用一个字段；列表中以「📄 本地 / 🔗 在线」区分，点击分别用系统默认程序或浏览器打开
- **手册管理**：状态栏「📁 手册管理」列出所有已存本地手册（大小、时间、被哪些库的哪些元件引用），点击文件名可预览；不再需要的一键删除，并自动清除引用它的元件上的手册标记
- **CLI 命令行工具**：`cmcli.exe` 与图形界面共用同一数据库，可查/增/改/删元件、库存增减、导入导出、手册管理，输出均为 JSON，方便 AI 与脚本自动入库。NSIS 安装包安装时可勾选"加入 PATH"（卸载自动撤销）。详见 [命令行使用说明.md](./命令行使用说明.md)
- **数据持久化**：SQLite 单文件数据库，位于用户数据目录（`%APPDATA%\com.componentmanager.app\components.db`），直接拷贝即可备份迁移

## 环境要求

| 依赖 | 版本 | 用途 |
|---|---|---|
| Node.js | ≥ 18 | 前端构建 |
| Rust（MSVC 工具链） | ≥ 1.70 | 后端编译 |
| Visual Studio C++ Build Tools | 含「使用 C++ 的桌面开发」工作负载 + Windows SDK | 编译链接 |
| WebView2 | Windows 10/11 自带 | 界面渲染 |

## 使用

```bash
npm install        # 安装前端依赖
npm run tauri dev    # 开发模式（自动起 Vite + 编译 Rust + 打开窗口）
npm run tauri build  # 打包 Windows 安装包（.msi / .exe）
```

打包产物位于 `src-tauri/target/release/bundle/` 下的 `msi/` 与 `nsis/` 目录。

## 项目结构

```
component-manager/
├── index.html / vite.config.js / package.json   # 前端工程
├── src/                        # React 前端
│   ├── main.jsx                # 入口
│   ├── App.jsx                 # 主界面：搜索 + 录入 + 列表 + 编辑弹窗
│   └── index.css               # 样式
└── src-tauri/                  # Rust 后端
    ├── src/main.rs             # Tauri 命令：增删改查 + 模糊搜索
    ├── Cargo.toml              # Rust 依赖（rusqlite 启用 bundled，自动编译内置 SQLite）
    ├── tauri.conf.json         # 应用与打包配置
    ├── capabilities/default.json  # 窗口权限
    └── icons/                  # 应用图标
```

## 后端命令一览

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `list_libraries` | — | `Vec<Library>` | 全部库（含元件数） |
| `create_library` | `name: String` | `id` | 新建库 |
| `rename_library` | `id, name` | `bool` | 重命名库 |
| `delete_library` | `id: i64` | `bool` | 删库并删除其全部元件 |
| `list_all_components` | `libraryId` | `Vec<Component>` | 该库全部元件 |
| `search_components` | `libraryId, keyword` | `Vec<Component>` | 库内四字段模糊搜索 |
| `add_component` | `libraryId, comp` | `id` | 新增到当前库 |
| `update_component` | `comp` | `bool` | 更新 |
| `delete_component` | `id: i64` | `bool` | 删除 |
| `db_path` | — | `String` | 数据库文件路径 |

## 后续可扩展

二维码标签、Excel 批量导入导出、元件照片、分类统计、低库存阈值自定义、数据手册本地缓存。
