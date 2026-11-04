import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { confirm, message } from "@tauri-apps/plugin-dialog";

const CATEGORIES = ["电阻", "电容", "二极管", "三极管", "IC", "模块", "其他"];

// 各类别的常用封装预设：点击即填入，仍可自己手输任意值
const PKG_PRESETS = {
  电阻: ["0201", "0402", "0603", "0805", "1206", "2512", "直插 1/4W"],
  电容: ["0402", "0603", "0805", "1210", "SOD-8", "铝电解 6.3x5.8", "直插铝电解"],
  二极管: ["SOD-123", "SOD-323", "SMA(DO-214AC)", "SMB(DO-214AA)", "DO-41", "DO-35 玻璃"],
  三极管: ["SOT-23", "SOT-323", "SOT-89", "TO-92", "TO-220", "TO-252(DPAK)", "TO-263"],
  IC: ["SOT-23-5", "SOP-8", "SOP-16", "TSSOP-20", "SSOP-28", "QFN-32", "LQFP48", "TQFP64", "DIP-8", "DIP-40"],
  模块: ["DIP 排针 2.54", "邮票孔", "ESP32 模组", "USB 型", "螺丝端子"],
  其他: ["0603", "0805", "SOT-23", "SOP-8", "DIP", "直插", "贴片"],
};
const LOW_STOCK = 5; // 数量低于此值高亮提醒

// 页面是否运行在 Tauri 应用窗口中（浏览器里打开 dev 页面时无法调用后端命令）
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Tauri 窗口（WebView2）里的原生 window.confirm/alert 不阻塞代码：
// 对话框异步弹出、JS 直接往下执行，会造成“先删除、后询问、取消无效”的 bug。
// 必须用 dialog 插件的 confirm —— 返回真正的 Promise，用户点“确定”后才继续。
async function askConfirm(text, kind = "question") {
  if (inTauri) return await confirm(text, { kind, title: "元件库存管理器" });
  return window.confirm(text); // 浏览器预览兜底
}

function notice(text, title = "元件库存管理器") {
  if (inTauri) return message(text, { title });
  return window.alert(text);
}

const EMPTY_FORM = {
  id: 0,
  name: "",
  category: "电阻",
  package: "",
  manufacturer: "",
  quantity: 0,
  location: "",
  datasheet_url: "",
  notes: "",
};

export default function App() {
  // 库状态：libs 列表，lib 为当前进入的库（null = 停留在选择页）
  const [libs, setLibs] = useState([]);
  const [lib, setLib] = useState(null);
  const [libBusy, setLibBusy] = useState(false);
  const [libError, setLibError] = useState("");
  const [newLibName, setNewLibName] = useState("");
  const [renaming, setRenaming] = useState(null); // { id, name }

  const [rows, setRows] = useState([]);
  const [keyword, setKeyword] = useState("");
  const [activeKw, setActiveKw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [sort, setSort] = useState({ key: "id", dir: -1 });
  const [modal, setModal] = useState(null); // { mode: "add" | "edit", form, err }
  const [ioMsg, setIoMsg] = useState(""); // 导入/导出结果提示

  async function doExport() {
    try {
      setBusy(true);
      setIoMsg("");
      const saved = await invoke("export_library", { libraryId: lib.id });
      if (saved) setIoMsg(`已导出：${saved}，把这个 JSON 文件发给同事即可`);
    } catch (e) {
      setIoMsg(`导出失败：${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function doImport() {
    try {
      setBusy(true);
      setIoMsg("");
      const res = await invoke("import_library", { libraryId: lib.id });
      if (res) {
        setIoMsg(`导入完成：新增 ${res.added} 条，跳过重复/无效 ${res.skipped} 条`);
        await refresh();
        await refreshLibs();
      }
    } catch (e) {
      setIoMsg(`导入失败：${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function refreshLibs() {
    if (!inTauri) return;
    try {
      setLibs(await invoke("list_libraries"));
    } catch (e) {
      setLibError(String(e));
    }
  }

  useEffect(() => {
    refreshLibs();
  }, []);

  /* ---------------- 库管理 ---------------- */

  async function createLib() {
    const name = newLibName.trim();
    if (!name) {
      setLibError("库名不能为空");
      return;
    }
    try {
      setLibBusy(true);
      setLibError("");
      await invoke("create_library", { name });
      setNewLibName("");
      await refreshLibs();
    } catch (e) {
      setLibError(String(e));
    } finally {
      setLibBusy(false);
    }
  }

  async function renameLib() {
    const name = renaming.name.trim();
    if (!name) {
      setLibError("库名不能为空");
      return;
    }
    try {
      setLibBusy(true);
      setLibError("");
      await invoke("rename_library", { id: renaming.id, name });
      setRenaming(null);
      await refreshLibs();
      setLib((cur) => (cur && cur.id === renaming.id ? { ...cur, name } : cur));
    } catch (e) {
      setLibError(String(e));
    } finally {
      setLibBusy(false);
    }
  }

  async function deleteLib(l) {
    const ok = await askConfirm(
      `确定删除库「${l.name}」及其全部 ${l.count} 条元件吗？此操作不可恢复。`,
      "warning"
    );
    if (!ok) return;
    try {
      setLibBusy(true);
      setLibError("");
      await invoke("delete_library", { id: l.id });
      await refreshLibs();
    } catch (e) {
      setLibError(String(e));
    } finally {
      setLibBusy(false);
    }
  }

  function enterLib(l) {
    setLibError("");
    setLib(l);
    setKeyword("");
    setActiveKw("");
    setError("");
    setRows([]);
  }

  function backToPicker() {
    setLib(null);
    setModal(null);
    refreshLibs();
  }

  /* ---------------- 元件 ---------------- */

  async function refresh(kw = activeKw) {
    if (!inTauri || !lib) {
      setRows([]);
      if (!inTauri) setError("当前在浏览器中打开，无法连接数据库，请在应用窗口中操作");
      return;
    }
    try {
      setBusy(true);
      setError("");
      const data = kw.trim()
        ? await invoke("search_components", { libraryId: lib.id, keyword: kw })
        : await invoke("list_all_components", { libraryId: lib.id });
      setRows(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // 进入某个库后自动加载列表
  useEffect(() => {
    if (lib) refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lib?.id]);

  function doSearch() {
    const kw = keyword.trim();
    setActiveKw(kw);
    refresh(kw);
  }

  function toggleSort(key) {
    setSort((s) => (s.key === key ? { key, dir: -s.dir } : { key, dir: 1 }));
  }

  const shown = useMemo(() => {
    const arr = [...rows];
    const { key, dir } = sort;
    arr.sort((a, b) => {
      const va = a[key] ?? "";
      const vb = b[key] ?? "";
      const cmp =
        typeof va === "number" ? va - vb : String(va).localeCompare(String(vb), "zh");
      return cmp * dir;
    });
    return arr;
  }, [rows, sort]);

  function openAdd() {
    setModal({ mode: "add", form: { ...EMPTY_FORM }, err: "" });
  }

  function openEdit(row) {
    setModal({ mode: "edit", form: { ...EMPTY_FORM, ...row }, err: "" });
  }

  function setField(field, value) {
    setModal((m) => ({ ...m, form: { ...m.form, [field]: value } }));
  }

  async function save() {
    const f = modal.form;
    if (!f.name.trim()) {
      setModal((m) => ({ ...m, err: "名称不能为空" }));
      return;
    }
    const comp = { ...f, quantity: Number(f.quantity) || 0 };
    try {
      setBusy(true);
      if (modal.mode === "add") {
        await invoke("add_component", { libraryId: lib.id, comp });
      } else {
        const ok = await invoke("update_component", { comp });
        if (!ok) throw new Error("记录不存在，可能已被删除");
      }
      setModal(null);
      await refresh();
      await refreshLibs(); // 更新库卡片上的数量
    } catch (e) {
      setModal((m) => ({ ...m, err: String(e) }));
    } finally {
      setBusy(false);
    }
  }

  async function remove(row) {
    const ok = await askConfirm(`确定删除元件「${row.name}」吗？`);
    if (!ok) return;
    try {
      setBusy(true);
      setError("");
      await invoke("delete_component", { id: row.id });
      await refresh();
      await refreshLibs();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function pickDatasheet() {
    try {
      setBusy(true);
      const res = await invoke("pick_datasheet"); // null = 用户取消
      if (!res) return;
      setField("datasheet_url", res.file_name);
      if (res.reused) {
        notice(
          `本地已有同名手册「${res.file_name}」。\n已跳过复制，直接复用现有文件（多个元件可共用同一份手册）。`
        );
      }
    } catch (e) {
      setModal((m) => ({ ...m, err: String(e) }));
    } finally {
      setBusy(false);
    }
  }

  async function openDatasheet(row) {
    const v = row.datasheet_url;
    if (!v) return;
    try {
      if (/^https?:\/\//i.test(v)) {
        await openUrl(v); // 网络链接走浏览器
      } else {
        await invoke("open_datasheet", { fileName: v }); // 本地手册交给系统默认程序
      }
    } catch (e) {
      setError(String(e));
    }
  }

  /* ---------------- 本地手册管理 ---------------- */

  const [dsMgr, setDsMgr] = useState(null); // { files, error, busy }

  const isLocalFile = (v) => v && !/^https?:\/\//i.test(v);

  async function openDsMgr() {
    setDsMgr({ files: [], error: "", busy: true });
    try {
      const files = await invoke("list_datasheets");
      setDsMgr({ files, error: "", busy: false });
    } catch (e) {
      setDsMgr({ files: [], error: String(e), busy: false });
    }
  }

  async function delDs(f) {
    const refTip =
      f.refs.length > 0
        ? `它仍被 ${f.refs.length} 个元件引用（${f.refs.slice(0, 3).join("、")}${
            f.refs.length > 3 ? "…" : ""
          }），删除后这些元件的手册引用会一并清除。`
        : "当前没有元件引用它，可放心删除。";
    const ok = await askConfirm(`确定删除本地手册「${f.file_name}」？${refTip}`, "warning");
    if (!ok) return;
    try {
      setDsMgr((m) => ({ ...m, busy: true, error: "" }));
      await invoke("delete_datasheet", { fileName: f.file_name });
      const files = await invoke("list_datasheets");
      setDsMgr({ files, error: "", busy: false });
      await refresh(); // 元件列表里的“手册”列状态可能变化
    } catch (e) {
      setDsMgr((m) => ({ ...m, busy: false, error: String(e) }));
    }
  }

  function fmtSize(bytes) {
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  /* ---------------- 渲染 ---------------- */

  if (!lib) {
    // 启动 / 切库时的库选择页
    return (
      <div className="app">
        {!inTauri && (
          <div className="browser-warn">
            此页面在浏览器中打开，只能预览界面，无法读写数据库。请在终端运行
            npm run tauri dev，等待编译完成、「元件库存管理器」应用窗口弹出后，在应用窗口内操作。
          </div>
        )}
        <div className="picker">
          <h2>选择元件库</h2>
          <p className="picker-sub">不同人 / 项目的元件分开存放，进入后可随时切换</p>

          {libError && <div className="lib-error">{libError}</div>}

          <div className="lib-grid">
            {libs.map((l) => (
              <div
                key={l.id}
                className="lib-card"
                onDoubleClick={() => enterLib(l)}
                title={`进入「${l.name}」`}
              >
                <div className="lib-name">{l.name}</div>
                <div className="lib-count">{l.count} 个元件</div>
                <div className="lib-actions">
                  <button className="primary" onClick={() => enterLib(l)}>
                    进入
                  </button>
                  <button onClick={() => setRenaming({ id: l.id, name: l.name })}>重命名</button>
                  <button className="danger" onClick={() => deleteLib(l)}>
                    删除
                  </button>
                </div>
              </div>
            ))}

            <div className="lib-card add">
              <input
                placeholder="新库名称，如：我的 / 张三的"
                value={newLibName}
                onChange={(e) => setNewLibName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && createLib()}
              />
              <button className="primary" onClick={createLib} disabled={libBusy}>
                ＋ 新建元件库
              </button>
            </div>
          </div>
        </div>

        {renaming && (
          <div className="modal-mask" onClick={() => setRenaming(null)}>
            <div className="modal small" onClick={(e) => e.stopPropagation()}>
              <header>重命名元件库</header>
              <div className="body">
                <input
                  className="full"
                  autoFocus
                  value={renaming.name}
                  onChange={(e) => setRenaming((r) => ({ ...r, name: e.target.value }))}
                  onKeyDown={(e) => e.key === "Enter" && renameLib()}
                />
              </div>
              <footer>
                <button onClick={() => setRenaming(null)}>取消</button>
                <button className="primary" onClick={renameLib} disabled={libBusy}>
                  确定
                </button>
              </footer>
            </div>
          </div>
        )}
      </div>
    );
  }

  // 已进入某个库：元件主界面
  const arrow = (key) => (sort.key === key ? (sort.dir > 0 ? " ↑" : " ↓") : "");

  return (
    <div className="app">
      {!inTauri && (
        <div className="browser-warn">
          此页面在浏览器中打开，只能预览界面，无法读写数据库。请在应用窗口中操作。
        </div>
      )}
      <div className="topbar">
        <h1>元件库存管理器</h1>
        <button className="lib-chip" onClick={backToPicker} title="返回库选择">
          📦 {lib.name} · 切换库
        </button>
        <div className="search-row">
          <input
            className="full"
            placeholder="在本库内搜 型号 / 类别 / 封装 / 位置..."
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && doSearch()}
          />
          <button className="primary" onClick={doSearch} disabled={busy}>
            搜索
          </button>
          {activeKw && (
            <button
              onClick={() => {
                setKeyword("");
                setActiveKw("");
                refresh("");
              }}
              title="显示全部"
            >
              ✕
            </button>
          )}
          <button onClick={openAdd}>＋ 新增元件</button>
        </div>
      </div>

      <div className="statusbar">
        <span>
          库「{lib.name}」共 {rows.length} 条
        </span>
        <button onClick={doExport} disabled={busy} title="导出为 JSON，发给同事导入">
          ⬆ 导出本库
        </button>
        <button onClick={doImport} disabled={busy} title="从 JSON 文件导入到当前库">
          ⬇ 导入到本库
        </button>
        <button onClick={openDsMgr} title="查看与删除已存本地的 PDF 手册">
          📁 手册管理
        </button>
        {ioMsg && <span className="io-msg">{ioMsg}</span>}
        {activeKw && <span>关键词：{activeKw}</span>}
        {busy && <span>处理中…</span>}
        {error && <span className="error">{error}</span>}
      </div>

      <div className="table-wrap">
        {shown.length === 0 && !busy ? (
          <div className="empty">
            {activeKw
              ? `库「${lib.name}」里没有匹配的元件`
              : `库「${lib.name}」还是空的，点击「＋ 新增元件」开始录入`}
          </div>
        ) : (
          <table>
            <thead>
              <tr>
                <th onClick={() => toggleSort("name")}>名称{arrow("name")}</th>
                <th onClick={() => toggleSort("category")}>类别{arrow("category")}</th>
                <th onClick={() => toggleSort("package")}>封装{arrow("package")}</th>
                <th onClick={() => toggleSort("manufacturer")}>制造商{arrow("manufacturer")}</th>
                <th onClick={() => toggleSort("quantity")}>数量{arrow("quantity")}</th>
                <th onClick={() => toggleSort("location")}>位置{arrow("location")}</th>
                <th>手册</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr key={r.id}>
                  <td>{r.name}</td>
                  <td>{r.category || <span className="muted">—</span>}</td>
                  <td className="mono">{r.package || <span className="muted">—</span>}</td>
                  <td>{r.manufacturer || <span className="muted">—</span>}</td>
                  <td className={r.quantity <= LOW_STOCK ? "qty-low" : ""}>{r.quantity}</td>
                  <td className="loc">{r.location || <span className="muted">—</span>}</td>
                  <td>
                    {r.datasheet_url ? (
                      <button
                        className="link-like"
                        onClick={() => openDatasheet(r)}
                        title={r.datasheet_url}
                      >
                        {isLocalFile(r.datasheet_url) ? "📄 本地" : "🔗 在线"}
                      </button>
                    ) : (
                      <span className="muted">—</span>
                    )}
                  </td>
                  <td>
                    <button onClick={() => openEdit(r)}>编辑</button>{" "}
                    <button className="danger" onClick={() => remove(r)}>
                      删除
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {modal && (
        <div className="modal-mask" onClick={() => setModal(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header>
              {modal.mode === "add" ? `新增元件（库：${lib.name}）` : `编辑：${modal.form.name}`}
            </header>
            <div className="body">
              <label className="req">名称</label>
              <input
                className="full"
                autoFocus
                value={modal.form.name}
                onChange={(e) => setField("name", e.target.value)}
                placeholder="如 STM32F103C8T6"
              />

              <label>类别</label>
              <select
                className="full"
                value={modal.form.category}
                onChange={(e) => setField("category", e.target.value)}
              >
                {CATEGORIES.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>

              <label>封装</label>
              <div className="pkg-area">
                <div className="pkg-tags">
                  {(PKG_PRESETS[modal.form.category] || []).map((p) => (
                    <button
                      type="button"
                      key={p}
                      className={"pkg-tag" + (modal.form.package === p ? " on" : "")}
                      title={modal.form.package === p ? "再次点击取消选择" : `填入 ${p}`}
                      onClick={() =>
                        setField("package", modal.form.package === p ? "" : p)
                      }
                    >
                      {p}
                    </button>
                  ))}
                </div>
                <input
                  className="full"
                  value={modal.form.package}
                  onChange={(e) => setField("package", e.target.value)}
                  placeholder="点选上方常用封装，或直接输入"
                />
              </div>

              <label>制造商</label>
              <input
                className="full"
                value={modal.form.manufacturer}
                onChange={(e) => setField("manufacturer", e.target.value)}
              />

              <label>数量</label>
              <input
                type="number"
                min="0"
                value={modal.form.quantity}
                onChange={(e) => setField("quantity", e.target.value)}
              />

              <label>位置</label>
              <input
                className="full"
                value={modal.form.location}
                onChange={(e) => setField("location", e.target.value)}
                placeholder="如 B05-R2-C08"
              />

              <label>数据手册</label>
              <div className="ds-area">
                <div className="file-row">
                  <button onClick={pickDatasheet} disabled={busy}>
                    📄 本地 PDF
                  </button>
                  {isLocalFile(modal.form.datasheet_url) ? (
                    <>
                      <span className="file-name" title={modal.form.datasheet_url}>
                        已存：{modal.form.datasheet_url}
                      </span>
                      <button className="link-like" onClick={() => setField("datasheet_url", "")}>
                        清除
                      </button>
                    </>
                  ) : (
                    <span className="muted">选后自动存入本地手册库</span>
                  )}
                </div>
                <input
                  className="full"
                  placeholder="或粘贴在线链接 https://..."
                  value={isLocalFile(modal.form.datasheet_url) ? "" : modal.form.datasheet_url}
                  onChange={(e) => setField("datasheet_url", e.target.value)}
                />
              </div>

              <label>备注</label>
              <textarea
                className="full"
                rows={3}
                value={modal.form.notes}
                onChange={(e) => setField("notes", e.target.value)}
              />

              {modal.err && <div className="err">{modal.err}</div>}
            </div>
            <footer>
              <button onClick={() => setModal(null)}>取消</button>
              <button className="primary" onClick={save} disabled={busy}>
                {modal.mode === "add" ? "保存" : "更新"}
              </button>
            </footer>
          </div>
        </div>
      )}

      {dsMgr && (
        <div className="modal-mask" onClick={() => setDsMgr(null)}>
          <div className="modal wide" onClick={(e) => e.stopPropagation()}>
            <header>
              本地手册管理
              <span className="muted" style={{ fontWeight: 400, fontSize: 12, marginLeft: 10 }}>
                存放目录：…\com.componentmanager.app\datasheets
              </span>
            </header>
            {dsMgr.busy ? (
              <div className="body plain">读取中…</div>
            ) : dsMgr.files.length === 0 ? (
              <div className="body plain">
                还没有存过本地手册。在新增/编辑元件时用「📄 本地 PDF」选择文件即可。
              </div>
            ) : (
              <div className="body ds-list">
                {dsMgr.error && <div className="err">{dsMgr.error}</div>}
                {dsMgr.files.map((f) => (
                  <div className="ds-item" key={f.file_name}>
                    <div className="ds-main">
                      <button
                        className="link-like ds-name"
                        onClick={() =>
                          invoke("open_datasheet", { fileName: f.file_name }).catch((e) =>
                            setDsMgr((m) => ({ ...m, error: String(e) }))
                          )
                        }
                        title="打开该文件"
                      >
                        {f.file_name}
                      </button>
                      <div className="ds-meta">
                        {fmtSize(f.size)} · {f.mtime} ·{" "}
                        {f.refs.length > 0 ? (
                          <span>被 {f.refs.length} 个元件引用：{f.refs.join("；")}</span>
                        ) : (
                          <span className="muted">无引用</span>
                        )}
                      </div>
                    </div>
                    <button className="danger" onClick={() => delDs(f)} disabled={dsMgr.busy}>
                      删除
                    </button>
                  </div>
                ))}
              </div>
            )}
            <footer>
              <button onClick={() => setDsMgr(null)}>关闭</button>
            </footer>
          </div>
        </div>
      )}
    </div>
  );
}
