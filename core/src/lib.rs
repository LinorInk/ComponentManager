//! cm-core —— 元件库存管理器存储层共享库
//! 图形界面（src-tauri）与命令行工具（cmcli）共同依赖本库：
//! 表结构、迁移、数据模型、查询与增删改、手册文件管理、导入导出格式只保留这一份实现。

pub mod appdir;
pub mod components;
pub mod datasheet;
pub mod db;
pub mod models;
pub mod transfer;

pub use models::{
    Component, DatasheetImport, DatasheetInfo, ImportResult, Library, LibraryFile, FILE_APP,
};
