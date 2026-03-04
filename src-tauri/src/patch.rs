use std::fs;

use crate::constants::INJECT_CODE;
use crate::models::PatchStatus;
use crate::utils::get_antigravity_base_path;

fn get_patch_target_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "vs/workbench/api/node/extensionHostProcess.js",
            "extensionHostProcess",
        ),
        (
            "vs/workbench/api/worker/extensionHostWorkerMain.js",
            "extensionHostWorker",
        ),
        ("main.js", "main"),
        ("vs/code/node/cliProcessMain.js", "cliProcessMain"),
    ]
}

#[tauri::command]
pub fn apply_patch(target_url: String) -> Result<String, String> {
    let base_path = get_antigravity_base_path().ok_or(
        "找不到 Antigravity IDE 安装路径。请确认是否安装在默认路径：%LOCALAPPDATA%\\Programs\\Antigravity",
    )?;

    let target_url = target_url.trim().trim_end_matches('/').to_string();
    if target_url.is_empty() {
        return Err("目标 URL 不能为空".to_string());
    }

    // Match only cloudcode-related URLs and previously-patched 127.0.0.1 targets.
    // Avoids replacing unrelated URLs (OAuth, telemetry, etc.)
    let url_pattern = regex::Regex::new(r"https://([a-zA-Z0-9.\-]*cloudcode[a-zA-Z0-9.\-]*\.googleapis\.com|127\.0\.0\.1:\d+)")
        .map_err(|e| format!("正则表达式编译失败: {}", e))?;

    let mut patched_count = 0;
    let mut errors = Vec::new();

    for (relative_path, name) in &get_patch_target_files() {
        let file_path = base_path.join(relative_path);
        if !file_path.exists() {
            continue;
        }

        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("读取 {} 失败: {}", name, e));
                continue;
            }
        };

        let backup_path = file_path.with_extension("js.bak");
        if !backup_path.exists() {
            if let Err(e) = fs::write(&backup_path, &content) {
                errors.push(format!("备份 {} 失败: {}", name, e));
                continue;
            }
        }

        let replaced_content = url_pattern
            .replace_all(&content, target_url.as_str())
            .to_string();

        let mut final_content = replaced_content;
        if !final_content.contains("NODE_TLS_REJECT_UNAUTHORIZED") {
            final_content = format!("{}{}", INJECT_CODE, final_content);
        }

        if final_content != content {
            if let Err(e) = fs::write(&file_path, &final_content) {
                errors.push(format!(
                    "写入 {} 失败: {}。请确认 IDE 已完全关闭，并尝试以管理员权限运行。",
                    name, e
                ));
                continue;
            }
            patched_count += 1;
        } else if content.contains(target_url.as_str()) {
            patched_count += 1;
        }
    }

    if patched_count == 0 {
        if !errors.is_empty() {
            Err(errors.join("\n"))
        } else {
            Err("没有找到任何 IDE 核心文件，或文件已经是最新状态。".to_string())
        }
    } else {
        let msg = if errors.is_empty() {
            format!(
                "成功处理 {} 个补丁文件（目标: {}）",
                patched_count, target_url
            )
        } else {
            format!(
                "部分成功（{} 个），存在以下错误：\n{}",
                patched_count,
                errors.join("\n")
            )
        };
        Ok(msg)
    }
}

#[tauri::command]
pub fn remove_patch() -> Result<String, String> {
    let base_path = get_antigravity_base_path().ok_or("找不到 Antigravity IDE 安装路径")?;

    let mut restored_count = 0;
    for (relative_path, name) in &get_patch_target_files() {
        let file_path = base_path.join(relative_path);
        let backup_path = file_path.with_extension("js.bak");

        if backup_path.exists() {
            fs::copy(&backup_path, &file_path).map_err(|e| format!("恢复 {} 失败: {}", name, e))?;
            fs::remove_file(&backup_path).ok();
            restored_count += 1;
        }
    }

    if restored_count == 0 {
        Err("没有找到可恢复的备份文件".to_string())
    } else {
        Ok(format!("成功恢复 {} 个文件", restored_count))
    }
}

#[tauri::command]
pub fn check_patch_status() -> PatchStatus {
    let base_path: std::path::PathBuf = match get_antigravity_base_path() {
        Some(p) => p,
        None => {
            return PatchStatus {
                applied: false,
                message: "未找到 Antigravity IDE".to_string(),
            }
        }
    };

    let main_js = base_path.join("main.js");
    if let Ok(content) = fs::read_to_string(&main_js) {
        if content.contains("NODE_TLS_REJECT_UNAUTHORIZED") {
            if let Ok(re) = regex::Regex::new(r"https?://127\.0\.0\.1:(\d+)") {
                if let Some(caps) = re.captures(&content) {
                    return PatchStatus {
                        applied: true,
                        message: format!("本地模式 (127.0.0.1:{})", &caps[1]),
                    };
                }
            }
            if let Ok(re) =
                regex::Regex::new(r"https?://([a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}(?::\d+)?)")
            {
                for caps in re.captures_iter(&content) {
                    let host = &caps[1];
                    if !host.contains("cloudcode")
                        && !host.contains("googleapis")
                        && !host.contains("google.com")
                    {
                        return PatchStatus {
                            applied: true,
                            message: format!("自定义 URL ({})", host),
                        };
                    }
                }
            }
            return PatchStatus {
                applied: true,
                message: "补丁已应用".to_string(),
            };
        }
    }

    PatchStatus {
        applied: false,
        message: "未应用补丁".to_string(),
    }
}

// ==================== Auto Accept ====================

const AUTO_ACCEPT_BEGIN: &str = "<!-- AB_AUTO_ACCEPT_START -->";
const AUTO_ACCEPT_END: &str = "<!-- AB_AUTO_ACCEPT_END -->";

fn get_workbench_html_path() -> Option<std::path::PathBuf> {
    let base = get_antigravity_base_path()?;
    let p = base.join("vs/code/electron-browser/workbench/workbench.html");
    if p.exists() { Some(p) } else { None }
}

fn get_jetski_agent_html_path() -> Option<std::path::PathBuf> {
    let base = get_antigravity_base_path()?;
    let p = base.join("vs/code/electron-browser/workbench/workbench-jetski-agent.html");
    if p.exists() { Some(p) } else { None }
}

/// Get all injectable HTML paths
fn get_all_injectable_html_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(p) = get_workbench_html_path() { paths.push(p); }
    if let Some(p) = get_jetski_agent_html_path() { paths.push(p); }
    paths
}

fn get_auto_accept_script() -> String {
    r#"<!-- AB_AUTO_ACCEPT_START -->
<script>
(function () {
  'use strict';
  var cachedCfg = { enabled:true, patterns:{ retry:false, run:true, apply:true, execute:true, confirm:false, allow:true, accept:true }, bannedCommands:['rm -rf /','rm -rf ~','rm -rf *','format c:','del /f /s /q','rmdir /s /q',':(){:|:&};:','dd if=','mkfs.','> /dev/sda','chmod -R 777 /'] };
  var proxyPort = 9530;
  try {
    var x0 = new XMLHttpRequest();
    x0.open('GET', '/port_config.txt', false);
    x0.send();
    if (x0.status === 200) { var pp = parseInt(x0.responseText.trim(), 10); if (pp > 0) proxyPort = pp; }
  } catch(e) {}
  function fetchCfg() {
    try {
      var x = new XMLHttpRequest();
      x.open('GET', 'https://127.0.0.1:' + proxyPort + '/auto-accept-config', true);
      x.timeout = 3000;
      x.onload = function() { if (x.status === 200) { try { cachedCfg = JSON.parse(x.responseText); } catch(e) {} } };
      x.send();
    } catch(e) {}
  }
  fetchCfg(); setInterval(fetchCfg, 5000);
  var REJECTS = ['skip','reject','cancel','close','refine','dismiss'];
  var clickedSet = typeof WeakSet!=='undefined' ? new WeakSet() : { add:function(){}, has:function(){return false;} };
  var cooldownUntil = 0;
  function isTarget(btn, cfg) {
    if (!cfg.enabled) return false;
    var t = (btn.textContent||'').trim().toLowerCase();
    if (!t || t.length>80 || btn.disabled) return false;
    if (btn.hasAttribute('aria-haspopup')) return false;
    if (REJECTS.some(function(r){ return t.indexOf(r)>=0; })) return false;
    var pats = cfg.patterns || {};
    return Object.keys(pats).some(function(k){ return pats[k] && t.indexOf(k)>=0; });
  }
  function nearbyText(btn) {
    var s='', el=btn.parentElement, d=0;
    while(el && d<8){ var sib=el.previousElementSibling, n=0; while(sib&&n<5){ if(/^(PRE|CODE)$/.test(sib.tagName)) s+=' '+sib.textContent; [].forEach.call(sib.querySelectorAll('pre,code'),function(e){s+=' '+e.textContent;}); sib=sib.previousElementSibling; n++; } if(s.length>20) break; el=el.parentElement; d++; }
    return s.toLowerCase();
  }
  function isBanned(text, cfg) { return (cfg.bannedCommands||[]).some(function(p){ return p && text.indexOf(p.toLowerCase())>=0; }); }
  var busy=false;
  function scan() {
    if(busy) return;
    var now=Date.now(); if(now<cooldownUntil) return;
    if(!cachedCfg.enabled) return; busy=true;
    var btns=[].slice.call(document.querySelectorAll('button')), idx=0, clicked=0;
    function next(){
      if(idx>=btns.length){ busy=false; if(clicked>0){ cooldownUntil=Date.now()+2000; } return; }
      var btn=btns[idx++];
      if(clickedSet.has(btn)){ next(); return; }
      if(!isTarget(btn,cachedCfg)){ next(); return; }
      var t=(btn.textContent||'').trim().toLowerCase();
      if((t.indexOf('run')>=0||t.indexOf('execute')>=0)&&isBanned(nearbyText(btn),cachedCfg)){ console.log('[AG-AutoAccept] blocked'); next(); return; }
      console.log('[AG-AutoAccept] clicking:', btn.textContent.trim());
      clickedSet.add(btn);
      btn.click(); clicked++;
      setTimeout(next,300);
    }
    next();
  }
  var timer; new MutationObserver(function(){ clearTimeout(timer); timer=setTimeout(scan,600); }).observe(document.documentElement,{childList:true,subtree:true});
  setTimeout(scan,2000);
  console.log('[AG-AutoAccept] ready (config from proxy)');
})();
</script>
<!-- AB_AUTO_ACCEPT_END -->"#.to_string()
}

#[tauri::command]
pub fn check_auto_accept_status() -> PatchStatus {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return PatchStatus { applied: false, message: "未找到 IDE HTML 文件".to_string() };
    }
    let mut injected = 0;
    let total = paths.len();
    for path in &paths {
        if let Ok(content) = fs::read_to_string(path) {
            if content.contains(AUTO_ACCEPT_BEGIN) {
                injected += 1;
            }
        }
    }
    if injected == total {
        PatchStatus { applied: true, message: format!("自动审批已开启 ({}/{})", injected, total) }
    } else if injected > 0 {
        PatchStatus { applied: true, message: format!("部分开启 ({}/{})", injected, total) }
    } else {
        PatchStatus { applied: false, message: "自动审批已关闭".to_string() }
    }
}

/// Fix CSP and inject script into a single HTML file
fn inject_auto_accept_into_file(html_path: &std::path::PathBuf) -> Result<String, String> {
    let content = fs::read_to_string(html_path)
        .map_err(|e| format!("读取 {} 失败: {}", html_path.display(), e))?;

    let file_name = html_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    if content.contains(AUTO_ACCEPT_BEGIN) {
        // Script already exists, but still check if CSP needs fixing
        let script_src_has_inline = if let Some(idx) = content.find("script-src") {
            let after = &content[idx..];
            let end = after.find(';').unwrap_or(after.len());
            after[..end].contains("'unsafe-inline'")
        } else {
            true
        };
        if !script_src_has_inline {
            if let Some(script_src_start) = content.find("script-src") {
                let after_script_src = &content[script_src_start..];
                let semicolon = after_script_src.find(';').unwrap_or(after_script_src.len());
                let script_src_section = &after_script_src[..semicolon];
                if let Some(eval_pos) = script_src_section.find("'unsafe-eval'") {
                    let abs_pos = script_src_start + eval_pos + "'unsafe-eval'".len();
                    let fixed = format!("{} 'unsafe-inline'{}", &content[..abs_pos], &content[abs_pos..]);
                    fs::write(html_path, fixed)
                        .map_err(|e| format!("修复 CSP 失败: {}", e))?;
                    return Ok(format!("{}: 已修复 CSP", file_name));
                }
            }
        }
        return Ok(format!("{}: 已存在", file_name));
    }

    // Backup
    let bak = html_path.with_extension("html.bak");
    if !bak.exists() {
        fs::write(&bak, &content).map_err(|e| format!("备份失败: {}", e))?;
    }

    // Fix CSP
    let patched_csp = {
        let script_src_has_inline = if let Some(idx) = content.find("script-src") {
            let after = &content[idx..];
            let end = after.find(';').unwrap_or(after.len());
            after[..end].contains("'unsafe-inline'")
        } else {
            false
        };
        if script_src_has_inline {
            content.clone()
        } else {
            if let Some(script_src_start) = content.find("script-src") {
                let after_script_src = &content[script_src_start..];
                let semicolon = after_script_src.find(';').unwrap_or(after_script_src.len());
                let script_src_section = &after_script_src[..semicolon];
                if let Some(eval_pos) = script_src_section.find("'unsafe-eval'") {
                    let abs_pos = script_src_start + eval_pos + "'unsafe-eval'".len();
                    format!("{} 'unsafe-inline'{}", &content[..abs_pos], &content[abs_pos..])
                } else {
                    content.clone()
                }
            } else {
                content.clone()
            }
        }
    };

    // Append script before </html> or at end
    let script = get_auto_accept_script();
    let final_content = if patched_csp.contains("</html>") {
        patched_csp.replace("</html>", &format!("\n{}\n</html>", script))
    } else {
        format!("{}\n{}", patched_csp, script)
    };

    fs::write(html_path, final_content)
        .map_err(|e| format!("写入 {} 失败: {}", file_name, e))?;

    Ok(format!("{}: 注入成功", file_name))
}

/// Remove auto-accept script from a single HTML file
fn remove_auto_accept_from_file(html_path: &std::path::PathBuf) -> Result<String, String> {
    let file_name = html_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let bak = html_path.with_extension("html.bak");
    if bak.exists() {
        fs::copy(&bak, html_path).map_err(|e| format!("恢复备份失败: {}", e))?;
        fs::remove_file(&bak).ok();
        return Ok(format!("{}: 已从备份恢复", file_name));
    }

    let content = fs::read_to_string(html_path)
        .map_err(|e| format!("读取失败: {}", e))?;

    if !content.contains(AUTO_ACCEPT_BEGIN) {
        return Ok(format!("{}: 无需撤销", file_name));
    }

    let start = content.find(AUTO_ACCEPT_BEGIN).unwrap_or(0);
    let end_marker = AUTO_ACCEPT_END;
    let cleaned = if let Some(end_pos) = content.find(end_marker) {
        let after = end_pos + end_marker.len();
        format!("{}{}", &content[..start], &content[after..])
    } else {
        content[..start].to_string()
    };

    fs::write(html_path, cleaned.trim_end())
        .map_err(|e| format!("写入失败: {}", e))?;

    Ok(format!("{}: 已移除", file_name))
}

#[tauri::command]
pub fn apply_auto_accept() -> Result<String, String> {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return Err("未找到 IDE HTML 文件，请确认 Antigravity IDE 已安装".to_string());
    }
    let mut results = Vec::new();
    for path in &paths {
        match inject_auto_accept_into_file(path) {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("❌ {}", e)),
        }
    }
    Ok(format!("✅ 自动审批注入完成：{}", results.join(" | ")))
}

#[tauri::command]
pub fn remove_auto_accept() -> Result<String, String> {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return Err("未找到 IDE HTML 文件".to_string());
    }
    let mut results = Vec::new();
    for path in &paths {
        match remove_auto_accept_from_file(path) {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("❌ {}", e)),
        }
    }
    Ok(format!("✅ 自动审批移除完成：{}", results.join(" | ")))
}

// ==================== Context Ring Indicator ====================

const CTX_RING_BEGIN: &str = "<!-- AG_CONTEXT_RING_START -->";
const CTX_RING_END: &str = "<!-- AG_CONTEXT_RING_END -->";

fn get_context_ring_script() -> String {
    let port = crate::proxy::load_port_config().unwrap_or(9527);
    format!(r#"<!-- AG_CONTEXT_RING_START -->
<script>
(function() {{
  'use strict';
  var PROXY_PORT = {port};
  var POLL_MS = 5000;
  var RING_SIZE = 22;
  var STROKE = 2.5;

  function fmtK(n) {{
    if (n >= 1000000) return (n/1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n/1000).toFixed(1) + 'k';
    return String(n);
  }}
  function ringColor(pct) {{
    // Smooth gradient: green(120) -> yellow(60) -> red(0) via HSL
    var hue;
    if (pct <= 0.5) {{
      // 0-50%: green(120) to yellow(60)
      hue = 120 - (pct / 0.5) * 60;
    }} else if (pct <= 0.8) {{
      // 50-80%: yellow(60) to red(0)
      hue = 60 - ((pct - 0.5) / 0.3) * 60;
    }} else {{
      hue = 0;
    }}
    var sat = 75 + pct * 15;
    var lit = 55 + (pct > 0.8 ? (pct - 0.8) * 25 : 0);
    return 'hsl(' + Math.round(hue) + ',' + Math.round(sat) + '%,' + Math.round(Math.min(lit, 60)) + '%)';
  }}

  var lastData = null;
  var ringEl = null;

  function makeSvgRing(size, stroke) {{
    var r = (size - stroke) / 2;
    var c = 2 * Math.PI * r;
    var ns = 'http://www.w3.org/2000/svg';
    var svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('width', size);
    svg.setAttribute('height', size);
    svg.setAttribute('viewBox', '0 0 ' + size + ' ' + size);
    svg.style.cssText = 'transform:rotate(-90deg);display:block;';
    var bg = document.createElementNS(ns, 'circle');
    bg.setAttribute('cx', size/2); bg.setAttribute('cy', size/2);
    bg.setAttribute('r', r); bg.setAttribute('fill', 'none');
    bg.setAttribute('stroke', 'rgba(255,255,255,0.13)');
    bg.setAttribute('stroke-width', stroke);
    svg.appendChild(bg);
    var fg = document.createElementNS(ns, 'circle');
    fg.setAttribute('cx', size/2); fg.setAttribute('cy', size/2);
    fg.setAttribute('r', r); fg.setAttribute('fill', 'none');
    fg.setAttribute('stroke', '#4ade80');
    fg.setAttribute('stroke-width', stroke);
    fg.setAttribute('stroke-dasharray', c.toFixed(2));
    fg.setAttribute('stroke-dashoffset', c.toFixed(2));
    fg.setAttribute('stroke-linecap', 'round');
    fg.style.cssText = 'transition:stroke-dashoffset 0.6s ease,stroke 0.6s ease;';
    svg.appendChild(fg);
    return {{ svg: svg, fg: fg, circumference: c }};
  }}

  function createRing() {{
    var wrap = document.createElement('div');
    wrap.id = 'ag-ctx-ring';
    wrap.style.cssText = 'display:flex;align-items:center;justify-content:center;position:relative;cursor:pointer;padding:1px;flex-shrink:0;';
    var ring = makeSvgRing(RING_SIZE, STROKE);
    wrap.appendChild(ring.svg);
    wrap._fg = ring.fg;
    wrap._circ = ring.circumference;
    var tip = document.createElement('div');
    tip.style.cssText = 'position:absolute;bottom:calc(100% + 8px);right:0;background:rgba(30,30,30,0.96);color:#e0e0e0;padding:5px 10px;border-radius:5px;font-size:11px;white-space:nowrap;pointer-events:none;opacity:0;transition:opacity 0.2s;z-index:99999;font-family:monospace;border:1px solid rgba(255,255,255,0.12);box-shadow:0 2px 8px rgba(0,0,0,0.4);';
    tip.textContent = '--/--';
    wrap.appendChild(tip);
    wrap._tip = tip;
    wrap.addEventListener('mouseenter', function() {{ tip.style.opacity = '1'; }});
    wrap.addEventListener('mouseleave', function() {{ tip.style.opacity = '0'; }});
    return wrap;
  }}

  function updateRingEl(el, data) {{
    if (!el || !el._fg || !el._tip) return;
    var used = data.input_tokens || 0;
    var max = data.max_context || 200000;
    var pct = Math.min(used / max, 1);
    var offset = el._circ * (1 - pct);
    el._fg.setAttribute('stroke-dashoffset', offset.toFixed(2));
    el._fg.setAttribute('stroke', ringColor(pct));
    var model = data.model || '';
    el._tip.textContent = fmtK(used) + ' / ' + fmtK(max) + (model ? ' (' + model + ')' : '');
  }}

  function tryInject() {{
    var mic = document.querySelector('button[aria-label="Record voice memo"]');
    if (!mic) return;
    var micWrapper = mic.parentElement;
    var buttonGroup = micWrapper && micWrapper.parentElement;
    if (!buttonGroup) return;
    var cs = window.getComputedStyle(buttonGroup);
    if (cs.display !== 'flex' || cs.flexDirection !== 'row') return;

    var existing = document.getElementById('ag-ctx-ring');
    // If ring exists and is correctly positioned right after micWrapper, do nothing
    if (existing && existing.previousSibling === micWrapper) return;
    // Remove if exists but in wrong position
    if (existing) existing.remove();

    ringEl = createRing();
    buttonGroup.insertBefore(ringEl, micWrapper.nextSibling);
    if (lastData) updateRingEl(ringEl, lastData);
  }}

  function poll() {{
    try {{
      var xhr = new XMLHttpRequest();
      xhr.open('GET', 'https://127.0.0.1:' + PROXY_PORT + '/context-info', true);
      xhr.timeout = 3000;
      xhr.onload = function() {{
        if (xhr.status === 200) {{
          try {{
            lastData = JSON.parse(xhr.responseText);
            tryInject();
            if (ringEl && document.getElementById('ag-ctx-ring')) updateRingEl(ringEl, lastData);
          }} catch(e) {{}}
        }}
      }};
      xhr.onerror = function() {{}};
      xhr.send();
    }} catch(e) {{}}
  }}

  setInterval(poll, POLL_MS);
  setTimeout(poll, 3000);

  var _mc = 0;
  new MutationObserver(function() {{
    if (_mc++ > 5000) return;
    tryInject();
  }}).observe(document.documentElement, {{ childList: true, subtree: true }});

  console.log('[AG-ContextRing] ready, port ' + PROXY_PORT);
}})();
</script>
<!-- AG_CONTEXT_RING_END -->"#)
}

#[tauri::command]
pub fn check_context_ring_status() -> PatchStatus {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return PatchStatus { applied: false, message: "未找到 IDE HTML 文件".to_string() };
    }
    let mut injected = 0;
    let total = paths.len();
    for path in &paths {
        if let Ok(content) = fs::read_to_string(path) {
            if content.contains(CTX_RING_BEGIN) {
                injected += 1;
            }
        }
    }
    if injected == total {
        PatchStatus { applied: true, message: format!("上下文统计已开启 ({}/{})", injected, total) }
    } else if injected > 0 {
        PatchStatus { applied: true, message: format!("部分开启 ({}/{})", injected, total) }
    } else {
        PatchStatus { applied: false, message: "上下文统计已关闭".to_string() }
    }
}

/// Inject context ring script into a single HTML file
fn inject_context_ring_into_file(html_path: &std::path::PathBuf) -> Result<String, String> {
    let content = fs::read_to_string(html_path)
        .map_err(|e| format!("读取 {} 失败: {}", html_path.display(), e))?;

    let file_name = html_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    if content.contains(CTX_RING_BEGIN) {
        // Already injected — update the script in case port changed
        if let Some(start) = content.find(CTX_RING_BEGIN) {
            if let Some(end_pos) = content.find(CTX_RING_END) {
                let after = end_pos + CTX_RING_END.len();
                let new_script = get_context_ring_script();
                let updated = format!("{}{}{}", &content[..start], new_script, &content[after..]);
                fs::write(html_path, updated)
                    .map_err(|e| format!("更新 {} 失败: {}", file_name, e))?;
                return Ok(format!("{}: 已更新", file_name));
            }
        }
        return Ok(format!("{}: 已存在", file_name));
    }

    // Fix CSP: ensure script-src has 'unsafe-inline'
    let patched_csp = {
        let script_src_has_inline = if let Some(idx) = content.find("script-src") {
            let after = &content[idx..];
            let end = after.find(';').unwrap_or(after.len());
            after[..end].contains("'unsafe-inline'")
        } else {
            false
        };
        if script_src_has_inline {
            content.clone()
        } else {
            if let Some(script_src_start) = content.find("script-src") {
                let after_script_src = &content[script_src_start..];
                let semicolon = after_script_src.find(';').unwrap_or(after_script_src.len());
                let script_src_section = &after_script_src[..semicolon];
                if let Some(eval_pos) = script_src_section.find("'unsafe-eval'") {
                    let abs_pos = script_src_start + eval_pos + "'unsafe-eval'".len();
                    format!("{} 'unsafe-inline'{}", &content[..abs_pos], &content[abs_pos..])
                } else {
                    content.clone()
                }
            } else {
                content.clone()
            }
        }
    };

    // Also ensure connect-src allows our proxy (https://127.0.0.1:*)
    let patched_connect = {
        if patched_csp.contains("https://127.0.0.1:*") {
            // Already has the wildcard for HTTPS 127.0.0.1
            patched_csp
        } else if let Some(connect_src_start) = patched_csp.find("connect-src") {
            let after = &patched_csp[connect_src_start..];
            let semicolon_pos = after.find(';').unwrap_or(after.len());
            let section = &after[..semicolon_pos];
            if section.contains("http://127.0.0.1:*") {
                // Add https variant after http variant
                let http_marker = "http://127.0.0.1:*";
                if let Some(marker_pos) = section.find(http_marker) {
                    let abs_pos = connect_src_start + marker_pos + http_marker.len();
                    format!("{}\n\t\t\t\t\thttps://127.0.0.1:*{}", &patched_csp[..abs_pos], &patched_csp[abs_pos..])
                } else {
                    patched_csp
                }
            } else {
                patched_csp
            }
        } else {
            patched_csp
        }
    };

    // Append script before </body> so it actually executes in Electron
    let script = get_context_ring_script();
    let final_content = if patched_connect.contains("</body>") {
        patched_connect.replace("</body>", &format!("\n{}\n</body>", script))
    } else if patched_connect.contains("</html>") {
        patched_connect.replace("</html>", &format!("\n{}\n</html>", script))
    } else {
        format!("{}\n{}", patched_connect, script)
    };

    fs::write(html_path, final_content)
        .map_err(|e| format!("写入 {} 失败: {}", file_name, e))?;

    Ok(format!("{}: 注入成功", file_name))
}

/// Remove context ring script from a single HTML file
fn remove_context_ring_from_file(html_path: &std::path::PathBuf) -> Result<String, String> {
    let file_name = html_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let content = fs::read_to_string(html_path)
        .map_err(|e| format!("读取失败: {}", e))?;

    if !content.contains(CTX_RING_BEGIN) {
        return Ok(format!("{}: 无需撤销", file_name));
    }

    let start = content.find(CTX_RING_BEGIN).unwrap_or(0);
    let cleaned = if let Some(end_pos) = content.find(CTX_RING_END) {
        let after = end_pos + CTX_RING_END.len();
        format!("{}{}", &content[..start], &content[after..])
    } else {
        content[..start].to_string()
    };

    fs::write(html_path, cleaned.trim_end())
        .map_err(|e| format!("写入失败: {}", e))?;

    Ok(format!("{}: 已移除", file_name))
}

#[tauri::command]
pub fn apply_context_ring() -> Result<String, String> {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return Err("未找到 IDE HTML 文件，请确认 Antigravity IDE 已安装".to_string());
    }
    let mut results = Vec::new();
    for path in &paths {
        match inject_context_ring_into_file(path) {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("❌ {}", e)),
        }
    }
    Ok(format!("✅ 上下文统计注入完成：{}", results.join(" | ")))
}

#[tauri::command]
pub fn remove_context_ring() -> Result<String, String> {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return Err("未找到 IDE HTML 文件".to_string());
    }
    let mut results = Vec::new();
    for path in &paths {
        match remove_context_ring_from_file(path) {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("❌ {}", e)),
        }
    }
    Ok(format!("✅ 上下文统计移除完成：{}", results.join(" | ")))
}

#[tauri::command]
pub fn get_context_ring_window(app: tauri::AppHandle) -> u64 {
    use tauri::Manager;
    if let Some(state) = app.try_state::<crate::models::AppState>() {
        *state.context_ring_window_secs.lock().unwrap()
    } else {
        15
    }
}

#[tauri::command]
pub fn set_context_ring_window(app: tauri::AppHandle, seconds: u64) -> String {
    use tauri::Manager;
    let secs = seconds.max(5).min(300); // clamp 5-300
    if let Some(state) = app.try_state::<crate::models::AppState>() {
        *state.context_ring_window_secs.lock().unwrap() = secs;
    }
    format!("滑动窗口设置为 {}s", secs)
}

#[tauri::command]
pub fn toggle_context_ring() -> Result<String, String> {
    let paths = get_all_injectable_html_paths();
    if paths.is_empty() {
        return Err("未找到 IDE HTML 文件".to_string());
    }
    // Check if already injected
    let any_injected = paths.iter().any(|p| {
        fs::read_to_string(p).map(|c| c.contains(CTX_RING_BEGIN)).unwrap_or(false)
    });
    if any_injected {
        // Remove
        let mut results = Vec::new();
        for path in &paths {
            match remove_context_ring_from_file(path) {
                Ok(msg) => results.push(msg),
                Err(e) => results.push(format!("❌ {}", e)),
            }
        }
        Ok(format!("已关闭：{}", results.join(" | ")))
    } else {
        // Inject
        let mut results = Vec::new();
        for path in &paths {
            match inject_context_ring_into_file(path) {
                Ok(msg) => results.push(msg),
                Err(e) => results.push(format!("❌ {}", e)),
            }
        }
        Ok(format!("已开启：{}", results.join(" | ")))
    }
}

#[tauri::command]
pub fn update_auto_accept_config(app: tauri::AppHandle, config_json: String) -> String {
    use tauri::Manager;
    if let Some(state) = app.try_state::<crate::models::AppState>() {
        *state.auto_accept_config.lock().unwrap() = config_json;
    }
    "ok".to_string()
}
