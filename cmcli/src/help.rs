//! 帮助文本：cmcli 独有，与命令表分开维护。

pub fn help_text() -> &'static str {
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
