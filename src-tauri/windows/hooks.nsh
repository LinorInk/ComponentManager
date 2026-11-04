; 元件库存管理器 —— NSIS 安装钩子
; 作用：安装结束时询问是否把命令行工具 cmcli 加入当前用户的 PATH；卸载前自动撤销。
; 实现说明：PATH 的写入/移除都交给随包安装的 cmcli.exe 完成，
;           它只写 HKEY_CURRENT_USER\Environment（无需管理员），且是幂等实现，
;           因此不会破坏系统 PATH，也不会重复写入同一条目。

!macro NSIS_HOOK_POSTINSTALL
  ; 静默安装（/S）不打扰：跳过询问，保持无人值守可用
  IfSilent cmcli_path_skip 0
  MessageBox MB_YESNO|MB_ICONQUESTION "是否把命令行工具 cmcli 加入环境变量 PATH？$\n$\n加入后，在任意终端都能直接使用 cmcli 命令快速录入元件。$\n（这项选择以后可以随时更改：运行 cmcli path-add 或 cmcli path-remove）" IDNO cmcli_path_skip
  nsExec::ExecToLog '"$INSTDIR\cmcli.exe" path-add'
  Pop $0
  cmcli_path_skip:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 趁 cmcli.exe 还没被删掉，撤销它当初加入 PATH 的那一条（没加过则什么也不做）
  IfFileExists "$INSTDIR\cmcli.exe" 0 cmcli_uninstall_skip
  nsExec::ExecToLog '"$INSTDIR\cmcli.exe" path-remove'
  Pop $0
  cmcli_uninstall_skip:
!macroend
