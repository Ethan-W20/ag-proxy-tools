// ==================== AG Proxy Tools - Frontend Logic ====================

function resolveInvokeBridge() {
  const invokeBridge = window.__TAURI__?.core?.invoke;
  return typeof invokeBridge === 'function' ? invokeBridge : mockInvoke;
}

function resolveListenBridge() {
  const listenBridge = window.__TAURI__?.event?.listen;
  if (typeof listenBridge === 'function') {
    return listenBridge;
  }
  return async () => () => { };
}

const invoke = resolveInvokeBridge();
const listen = resolveListenBridge();


// ==================== State Management ====================

let state = {
  accounts: [],
  currentIdx: -1,
  proxyRunning: false,
  totalRequests: 0,
  totalErrors: 0,
  tokenStats: {
    total_input: 0,
    total_output: 0,
    total_cache_read: 0,
    total_cache_creation: 0,
    total_tokens: 0,
    total_requests: 0,
    total_errors: 0,
  },
  logs: [],
  patchApplied: false,
  certInstalled: false,
  providers: [],
};

const CURRENT_ACCOUNT_EMAIL_KEY = 'ag-current-account-email';
const CURRENT_ACCOUNT_INDEX_KEY = 'ag-current-account-index';
const UI_LANGUAGE_KEY = 'ag-ui-language';
let credentialsLoading = false;
let credentialsLoadRunId = 0;
const ACCOUNT_LOAD_UI_INTERVAL_MS = 80;
const ACCOUNT_LOAD_UI_WARMUP_MS = 220;
let streamingAccountsUiMode = false;
let dashboardMetricsRefreshTimer = null;
let dashboardMetricsLoading = false;
let currentUiLanguage = 'zh';

const UI_STATIC_TEXT = {
  // Navigation
  navDashboardLabel: { zh: '仪表盘', en: 'Dashboard' },
  navAccountsLabel: { zh: '账号管理', en: 'Accounts' },
  navProvidersLabel: { zh: 'AI 供应商', en: 'AI Providers' },
  navToolsLabel: { zh: '工具箱', en: 'Toolbox' },
  navTokenStatsLabel: { zh: 'Token 统计', en: 'Token Stats' },
  navLogsLabel: { zh: '运行日志', en: 'Logs' },
  navSettingsLabel: { zh: '设置', en: 'Settings' },
  // Dashboard page
  dashPageTitle: { zh: '仪表盘', en: 'Dashboard' },
  dashPageDesc: { zh: 'AG Proxy 运行状态总览', en: 'AG Proxy runtime status overview' },
  dashProxyStatusLabel: { zh: '代理状态', en: 'Proxy Status' },
  dashAccountLabel: { zh: '账号总览', en: 'Accounts' },
  dashRequestLabel: { zh: '请求统计', en: 'Requests' },
  dashTokenLabel: { zh: 'Token 使用（M）', en: 'Token Usage (M)' },
  dashQuickStartTitle: { zh: '快速开始', en: 'Quick Start' },
  dashPatchBtn: { zh: '应用IDE补丁', en: 'Apply IDE Patch' },
  dashCertBtn: { zh: '导入证书', en: 'Import Certificate' },
  dashCurrentAcctTitle: { zh: '当前账号', en: 'Current Account' },
  dashAbnormalLabel: { zh: '异常账号', en: 'Abnormal' },
  dashErrorLabel: { zh: '错误请求', en: 'Errors' },
  dashInputLabel: { zh: '输入', en: 'Input' },
  dashOutputLabel: { zh: '输出', en: 'Output' },
  // Accounts page
  acctPageTitle: { zh: '账号管理', en: 'Account Management' },
  acctAddBtn: { zh: '添加账号', en: 'Add Account' },
  importFilesBtn: { zh: '📁 导入凭证文件', en: '📁 Import Credential Files' },
  importTokenBtn: { zh: '🔑 粘贴 Refresh Token', en: '🔑 Paste Refresh Token' },
  importOAuthBtn: { zh: '🌐 Google 登录', en: '🌐 Google Login' },
  acctQueryQuotaBtn: { zh: '查询额度', en: 'Query Quotas' },
  emptyAcctTitle: { zh: '暂无账号', en: 'No Accounts' },
  emptyAcctDesc: { zh: '点击「加载凭证」导入账号文件', en: 'Click "Add Account" to import credential files' },
  // Providers page
  provPageTitle: { zh: 'AI 供应商', en: 'AI Providers' },
  provAddBtn: { zh: '添加供应商', en: 'Add Provider' },
  emptyProvTitle: { zh: '暂无 AI 供应商', en: 'No AI Providers' },
  emptyProvDesc: { zh: '点击「添加供应商」配置第三方 API 供应商', en: 'Click "Add Provider" to configure third-party API providers' },
  provFormNameLabel: { zh: '供应商名称', en: 'Provider Name' },
  provFormProtoLabel: { zh: 'API 协议', en: 'API Protocol' },
  provFormBaseUrlHint: { zh: 'API 端点的基础地址（不含 /chat/completions 等路径）', en: 'Base URL for API endpoint (without /chat/completions path)' },
  provFormMappingLabel: { zh: '模型映射', en: 'Model Mapping' },
  provFormOptional: { zh: '（可选）', en: '(optional)' },
  provFormAddMappingBtn: { zh: '添加映射', en: 'Add Mapping' },
  provFormMappingHint: { zh: '将反重力请求的模型名称映射到供应商实际的模型名称', en: 'Map requested model names to the provider\'s actual model names' },
  provFormCancelBtn: { zh: '取消', en: 'Cancel' },
  provFormSaveBtn: { zh: '保存', en: 'Save' },
  // Tools page
  toolsPageTitle: { zh: '工具箱', en: 'Toolbox' },
  toolsPageDesc: { zh: 'IDE 补丁与证书管理', en: 'IDE patch and certificate management' },
  toolPatchTitle: { zh: 'IDE 补丁管理', en: 'IDE Patch Management' },
  toolApplyPatchBtn: { zh: '应用补丁', en: 'Apply Patch' },
  toolRemovePatchBtn: { zh: '撤销补丁', en: 'Remove Patch' },
  toolEditPatchBtn: { zh: '修改补丁', en: 'Edit Patch' },
  toolCertTitle: { zh: '证书管理', en: 'Certificate Management' },
  toolImportCertBtn: { zh: '导入证书', en: 'Import Certificate' },
  toolRemoveCertBtn: { zh: '卸载证书', en: 'Uninstall Certificate' },
  toolPatchDesc: { zh: '修改 Antigravity IDE 的核心文件，将 API 请求重定向到本地代理', en: 'Modify IDE core files to redirect API requests to the local proxy' },
  toolCertDesc: { zh: '管理本地代理 HTTPS 自签证书，导入到系统受信任的根证书存储', en: 'Manage local proxy HTTPS certificates and import to trusted root store' },
  // Patch target modal
  patchModalTitle: { zh: '修改补丁目标', en: 'Edit Patch Target' },
  patchModalUrlLabel: { zh: '补丁目标 URL', en: 'Patch Target URL' },
  patchModalResetBtn: { zh: '恢复默认', en: 'Reset Default' },
  patchModalCancelBtn: { zh: '取消', en: 'Cancel' },
  patchModalSaveBtn: { zh: '保存', en: 'Save' },
  // Token Stats
  tokenStatsEmptyData: { zh: '暂无数据，开始使用代理后将自动统计', en: 'No data yet. Stats will appear once proxy is used.' },
  // Logs page
  logsPageTitle: { zh: '运行日志', en: 'Logs' },
  logModeAllLabel: { zh: '全部日志', en: 'All Logs' },
  logModeErrorLabel: { zh: '仅错误', en: 'Errors Only' },
  logModeFlowLabel: { zh: '请求追踪', en: 'Request Tracing' },
  logClearBtnLabel: { zh: '清空', en: 'Clear' },
  flowLegendSuccess: { zh: '成功 (2xx)', en: 'Success (2xx)' },
  flowLegendError: { zh: '失败 (4xx/5xx)', en: 'Fail (4xx/5xx)' },
  flowLegendPending: { zh: '连接错误', en: 'Conn Error' },
  flowLegendReq: { zh: '→ 请求方向', en: '→ Request' },
  flowLegendResp: { zh: '← 响应方向', en: '← Response' },
  emptyFlowTitle: { zh: '暂无请求记录', en: 'No Requests' },
  emptyFlowDesc: { zh: '启动代理后，请求的完整链路追踪将显示在这里', en: 'Start proxy to see request tracing here' },
  // Dashboard proxy button
  proxyStartBtnLabel: { zh: '启动代理', en: 'Start Proxy' },
  // Token Stats page
  tokenStatsPageTitle: { zh: 'Token 使用统计', en: 'Token Usage Stats' },
  tokenStatsPageDesc: { zh: '跟踪总输入、输出、缓存 Token 及各账号用量', en: 'Track total input, output, cache tokens and per-account usage' },
  tokenStatsDetailTitle: { zh: '各账号用量明细', en: 'Per-Account Usage Details' },
  tokenStatsRefreshBtn: { zh: '刷新', en: 'Refresh' },
  tokenStatsResetBtn: { zh: '重置', en: 'Reset' },
  tokenStatsTh1: { zh: '账号', en: 'Account' },
  tokenStatsTh2: { zh: '输入(M)', en: 'Input(M)' },
  tokenStatsTh3: { zh: '输出(M)', en: 'Output(M)' },
  tokenStatsTh4: { zh: '缓存读取(M)', en: 'Cache Read(M)' },
  tokenStatsTh5: { zh: '总计(M)', en: 'Total(M)' },
  tokenStatsTh6: { zh: '请求', en: 'Requests' },
  statsInputLabel: { zh: '输入 Token（M）', en: 'Input Token (M)' },
  statsOutputLabel: { zh: '输出 Token（M）', en: 'Output Token (M)' },
  statsCacheLabel: { zh: '缓存读取（M）', en: 'Cache Read (M)' },
  statsRequestsLabel: { zh: '请求次数', en: 'Total Requests' },
  // Settings page
  settingsPageTitle: { zh: '设置', en: 'Settings' },
  settingsPageDesc: { zh: '应用程序配置', en: 'Application settings' },
  settingsTransportTitle: { zh: '传输与请求策略', en: 'Transport and request strategy' },
  settAutoStartLabel: { zh: '启动时自动开启代理', en: 'Auto-start proxy on launch' },
  settAutoStartDesc: { zh: '开启后软件启动时自动启动代理服务，无需手动点击', en: 'Automatically start proxy when app launches' },
  settPortLabel: { zh: '代理端口', en: 'Proxy Port' },
  settPortDesc: { zh: '本地代理监听端口，用于接收 IDE 请求并转发到上游', en: 'Local proxy listen port for IDE request forwarding' },
  settOfficialLsLabel: { zh: '官方 LS 转发', en: 'Official LS Forwarding' },
  settOfficialLsDesc: { zh: '通过官方 Language Server 转发请求到 Google Cloud（推荐开启）', en: 'Forward requests via Official Language Server to Google Cloud (recommended)' },
  settHttpProtocolLabel: { zh: '上游 HTTP 协议', en: 'Upstream HTTP Protocol' },
  settAutoProtoLabel: { zh: '自动', en: 'Auto' },
  settCapacityLabel: { zh: '模型互补', en: 'Capacity Failover' },
  settCapacityDesc: { zh: '仅针对 opus/sonnet thinking 自动重试并互切', en: 'Auto-retry with complementary model (opus/sonnet thinking)' },
  settUpstreamLabel: { zh: '上游服务器', en: 'Upstream Server' },
  settUpstreamDesc: { zh: 'Google Cloud Code PA API 端点', en: 'Google Cloud Code PA API endpoint' },
  settPassthroughLabel: { zh: '透传模式', en: 'Header Passthrough' },
  settPassthroughDesc: { zh: '关闭后，可解决403身份验证问题', en: 'Disable to fix 403 auth issues' },
  settRoutingLabel: { zh: '路由策略', en: 'Routing Strategy' },
  settFillLabel: { zh: '填充', en: 'Fill' },
  settRoundRobinLabel: { zh: '轮询', en: 'Round Robin' },
  settThresholdLabel: { zh: '额度切换阈值', en: 'Quota Switch Threshold' },
  settAutoRefreshLabel: { zh: '自动刷新额度', en: 'Auto-refresh Quota' },
  settAutoRefreshDesc: { zh: '开启后每 2 分钟自动查询当前账号额度，确保额度信息准确', en: 'Query quota every 2 minutes to keep data accurate' },
  settingsAppearanceTitle: { zh: '外观', en: 'Appearance' },
  themeSettingLabel: { zh: '主题颜色', en: 'Theme' },
  themeSettingDesc: { zh: '切换应用外观', en: 'Switch app appearance' },
  themeDarkText: { zh: '深色', en: 'Dark' },
  themeLightText: { zh: '浅色', en: 'Light' },
  themeSystemText: { zh: '跟随系统', en: 'System' },
  uiLanguageLabel: { zh: '界面语言', en: 'Language' },
  uiLanguageDesc: { zh: '切换中文与 English', en: 'Switch Chinese and English' },
  uiLangBtnZh: { zh: '中文', en: 'Chinese' },
  uiLangBtnEn: { zh: 'English', en: 'English' },
  // Upstream server dropdown
  upstreamDisplayText: { zh: '固定回退：sandbox → daily → prod（推荐）', en: 'Fixed fallback: sandbox → daily → prod (recommended)' },
  upstreamOptSandbox: { zh: '固定回退：sandbox → daily → prod（推荐）', en: 'Fixed fallback: sandbox → daily → prod (recommended)' },
  upstreamOptCustom: { zh: '🌐 自定义地址...', en: '🌐 Custom address...' },
  upstreamCustomHint: { zh: '输入自定义上游服务器地址（不含 https://）', en: 'Enter custom upstream server address (without https://)' },
  // Sidebar, log, patch modal
  sidebarProxyStatus: { zh: '代理未启动', en: 'Proxy not started' },
  initialLogLine: { zh: '[系统] 等待代理启动...', en: '[System] Waiting for proxy to start...' },
  patchModalHint: { zh: '留空则默认使用本地代理地址：', en: 'Leave empty to use local proxy address:' },
};

function uiText(zh, en) {
  return currentUiLanguage === 'en' ? en : zh;
}

// Backend message translation map: Chinese → English
// Rust backend always sends Chinese; JS translates when language is English
const BACKEND_MSG_MAP = {
  // Proxy lifecycle
  '代理已启动': 'Proxy started',
  '代理停止中': 'Proxy stopping',
  '代理已停止': 'Proxy stopped',
  '代理已在运行': 'Proxy is already running',
  '代理未运行': 'Proxy is not running',
  '已发送代理停止信号': 'Proxy shutdown signal sent',
  // Gateway health
  '网关健康': 'Gateway healthy',
  // Token stats
  'Token 统计已重置': 'Token stats reset',
  'Token 统计已写入磁盘': 'Token stats flushed to disk',
  // Flow node names (used in request tracing UI)
  '客户端': 'Client',
  '本地代理': 'Local Proxy',
  '网关': 'Gateway',
  'LS桥接': 'LS Bridge',
  '上游官方': 'Upstream',
  // Log messages
  '连接处理失败': 'Connection handler error',
  '读取请求体失败': 'Failed to read request body',
  // Account management
  '无效的账号索引': 'Invalid account index',
  '手动禁用': 'Manually disabled',
  '正在验证 Refresh Token...': 'Verifying Refresh Token...',
  '正在获取用户信息...': 'Retrieving user info...',
  'Refresh Token 不能为空': 'Refresh Token cannot be empty',
  '正在打开浏览器授权...': 'Opening browser for authorization...',
  '正在换取 Token...': 'Exchanging Token...',
  '没有可用账号': 'No available accounts',
  // Quota
  'API 返回 HTTP 403 Forbidden': 'API returned HTTP 403 Forbidden',
  // Patch
  '目标 URL 不能为空': 'Target URL cannot be empty',
  '补丁已应用': 'Patch applied',
  '未应用补丁': 'Patch not applied',
  '未找到 Antigravity IDE': 'Antigravity IDE not found',
  '没有找到可恢复的备份文件': 'No recoverable backup files found',
  // Local gateway
  '内置网关已在运行': 'Built-in gateway is already running',
  '内置网关已停止': 'Built-in gateway stopped',
  '网关监听地址不能为空': 'Gateway listen address cannot be empty',
  '内置网关仅支持 http 监听地址': 'Built-in gateway only supports http listen address',
  '网关地址必须包含主机名': 'Gateway address must include a hostname',
  '网关地址必须包含端口': 'Gateway address must include a port',
  '网关地址不能包含路径': 'Gateway address cannot contain a path',
  '网关没有可用上游地址': 'Gateway has no available upstream address',
  '网关响应无效': 'Gateway response invalid',
  // Provider
  '供应商配置已保存': 'Provider config saved',
};

// Prefix-based translation patterns for messages with dynamic content
const BACKEND_MSG_PREFIX_MAP = [
  ['代理已启动，监听', 'Proxy started, listening on'],
  ['代理端口已保存:', 'Proxy port saved:'],
  ['无效代理端口:', 'Invalid proxy port:'],
  ['保存的端口无效:', 'Stored port is invalid:'],
  ['无效路由策略:', 'Invalid routing strategy:'],
  ['无效额度阈值:', 'Invalid quota threshold:'],
  ['网关返回 HTTP', 'Gateway returned HTTP'],
  ['网关健康检查失败:', 'Gateway health check failed:'],
  ['创建 HTTP 客户端失败:', 'Failed to create HTTP client:'],
  ['绑定 127.0.0.1:', 'Failed to bind 127.0.0.1:'],
  ['连接处理失败:', 'Connection handler error:'],
  ['Token 请求失败:', 'Token request failed:'],
  ['Token 响应解析失败:', 'Token response parse failed:'],
  ['Token 刷新失败:', 'Token refresh failed:'],
  ['网关地址不能为空', 'Gateway address cannot be empty'],
  ['网关地址必须以', 'Gateway address must start with'],
  ['自定义上游地址不能为空', 'Custom upstream address cannot be empty'],
  ['读取请求体失败:', 'Failed to read request body:'],
  ['userinfo 客户端初始化失败:', 'Userinfo client init failed:'],
  ['userinfo 转发失败:', 'Userinfo forward failed:'],
  ['账号鉴权失败', 'Account auth failed'],
  ['解析上游目标失败:', 'Failed to resolve upstream targets:'],
  // Account management
  ['保存账号文件失败:', 'Failed to save account file:'],
  ['已导入:', 'Imported:'],
  ['获取用户信息失败:', 'Failed to get user info:'],
  ['解析失败:', 'Parse failed:'],
  ['账号导入成功:', 'Account imported:'],
  ['导入完成:', 'Import completed:'],
  ['导入账号：未能自动获取 project ID，将在首次请求时重试', 'Import: could not auto-detect project ID, will retry on first request'],
  ['OAuth 登录：未能自动获取 project ID，将在首次请求时重试', 'OAuth login: could not auto-detect project ID, will retry on first request'],
  ['OAuth 登录成功:', 'OAuth login successful:'],
  ['无法启动回调服务:', 'Failed to start callback server:'],
  ['等待授权回调', 'Waiting for auth callback'],
  ['OAuth 授权超时', 'OAuth authorization timed out'],
  ['Token 交换失败:', 'Token exchange failed:'],
  ['账号', 'Account'],
  ['保存', 'Save'],
  // Quota
  ['额度查询失败:', 'Quota query failed:'],
  ['额度响应解析失败:', 'Quota response parse failed:'],
  ['额度查询成功:', 'Quota query succeeded:'],
  ['额度 API 对账号', 'Quota API for account'],
  ['开始查询账号额度:', 'Querying account quota:'],
  ['开始查询', 'Querying'],
  ['额度查询重试', 'Quota query retry'],
  ['额度查询完成', 'Quota query completed'],
  ['额度查询失败：刷新失败', 'Quota query failed: refresh failed'],
  ['网络错误:', 'Network error:'],
  // Patch
  ['找不到 Antigravity IDE 安装路径', 'Cannot find Antigravity IDE installation path'],
  ['正则表达式编译失败:', 'Regex compilation failed:'],
  ['成功处理', 'Successfully processed'],
  ['成功恢复', 'Successfully restored'],
  ['部分成功', 'Partially succeeded'],
  ['没有找到任何 IDE 核心文件', 'No IDE core files found'],
  ['本地模式', 'Local mode'],
  ['自定义 URL', 'Custom URL'],
  // Local gateway
  ['网关地址格式无效:', 'Invalid gateway address format:'],
  ['网关地址缺少主机名', 'Gateway address missing hostname'],
  ['网关地址缺少端口', 'Gateway address missing port'],
  ['启动内置网关失败:', 'Failed to start built-in gateway:'],
  ['内置网关异常退出:', 'Built-in gateway exited abnormally:'],
  ['网关转发失败:', 'Gateway forward failed:'],
  // Provider
  ['供应商请求失败:', 'Provider request failed:'],
  ['供应商返回错误', 'Provider returned error'],
  ['供应商错误', 'Provider error'],
  ['供应商响应完成', 'Provider response completed'],
  ['返回 SSE 响应给 IDE', 'Returning SSE response to IDE'],
];

// Translate a backend message based on current UI language
function translateBackendMsg(msg) {
  if (!msg || currentUiLanguage !== 'en') return msg;
  // Exact match
  if (BACKEND_MSG_MAP[msg]) return BACKEND_MSG_MAP[msg];
  // Prefix match (replace Chinese prefix with English, keep dynamic suffix)
  for (const [zhPrefix, enPrefix] of BACKEND_MSG_PREFIX_MAP) {
    if (msg.startsWith(zhPrefix)) {
      return enPrefix + msg.slice(zhPrefix.length);
    }
  }
  return msg;
}

function applyUiLanguage() {
  Object.entries(UI_STATIC_TEXT).forEach(([id, pair]) => {
    const el = document.getElementById(id);
    if (el) {
      el.textContent = currentUiLanguage === 'en' ? pair.en : pair.zh;
    }
  });

  // Switch tooltip titles based on current language
  const titleKey = currentUiLanguage === 'en' ? 'titleEn' : 'titleZh';
  document.querySelectorAll('[data-title-zh]').forEach(el => {
    const val = el.dataset[titleKey];
    if (val) el.title = val;
  });

  document.documentElement.lang = currentUiLanguage === 'en' ? 'en' : 'zh-CN';
  document.getElementById('uiLangBtnZh')?.classList.toggle('active', currentUiLanguage === 'zh');
  document.getElementById('uiLangBtnEn')?.classList.toggle('active', currentUiLanguage === 'en');
  updateDashboard();
}

function setUiLanguage(lang) {
  currentUiLanguage = lang === 'en' ? 'en' : 'zh';
  localStorage.setItem(UI_LANGUAGE_KEY, currentUiLanguage);
  applyUiLanguage();
  showToast(
    currentUiLanguage === 'en' ? 'Language switched to English' : '语言已切换为中文',
    'success',
  );
}

function restoreUiLanguage() {
  const saved = localStorage.getItem(UI_LANGUAGE_KEY);
  currentUiLanguage = saved === 'en' ? 'en' : 'zh';
  applyUiLanguage();
}

// ==================== Page Navigation ====================

function switchPage(pageName) {
  document.querySelectorAll('.page').forEach(p => p.classList.remove('active'));
  document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));

  const page = document.getElementById('page-' + pageName);
  const nav = document.querySelector(`.nav-item[data-page="${pageName}"]`);

  if (page) page.classList.add('active');
  if (nav) nav.classList.add('active');

  // Lazy-load token stats only when user opens the stats page.
  if (pageName === 'dashboard') {
    refreshDashboardMetrics({ silent: true });
  }
  if (pageName === 'token-stats') {
    loadTokenStats();
  }
  if (pageName === 'tools') {
    checkPatchStatus();
    checkCertStatus();
  }
}

// ==================== Toast Notification ====================

function showToast(message, type = 'info') {
  message = translateBackendMsg(message);
  const container = document.getElementById('toastContainer');
  if (!container) {
    console.log(`[toast:${type}] ${message}`);
    return;
  }
  const toast = document.createElement('div');
  toast.className = `toast ${type}`;

  const icons = {
    success: '✅',
    error: '❌',
    info: 'ℹ️',
    warning: '⚠️',
  };

  toast.innerHTML = `<span>${icons[type] || 'ℹ️'}</span><span>${escapeHtml(message)}</span>`;
  container.appendChild(toast);

  setTimeout(() => {
    toast.classList.add('removing');
    setTimeout(() => toast.remove(), 300);
  }, 3000);
}

// ==================== IDE Patch ====================

const PATCH_TARGET_URL_KEY = 'ag-patch-target-url';
const LEGACY_PATCH_MODE_KEY = 'ag-patch-mode';
const LEGACY_PATCH_CUSTOM_URL_KEY = 'ag-patch-custom-url';

function normalizePatchTargetUrl(raw) {
  let url = (raw || '').trim();
  if (!url) return '';
  if (!/^https?:\/\//i.test(url)) {
    url = 'https://' + url;
  }
  return url.replace(/\/+$/, '');
}

function getDefaultPatchTargetUrl() {
  const port = parseInt(document.getElementById('proxyPort')?.value, 10) || 9527;
  return `https://127.0.0.1:${port}`;
}

function migrateLegacyPatchTargetConfig() {
  try {
    const current = normalizePatchTargetUrl(localStorage.getItem(PATCH_TARGET_URL_KEY) || '');
    if (!current) {
      const legacyMode = localStorage.getItem(LEGACY_PATCH_MODE_KEY);
      const legacyCustom = normalizePatchTargetUrl(localStorage.getItem(LEGACY_PATCH_CUSTOM_URL_KEY) || '');
      if (legacyMode === 'custom' && legacyCustom) {
        localStorage.setItem(PATCH_TARGET_URL_KEY, legacyCustom);
      }
    }
    localStorage.removeItem(LEGACY_PATCH_MODE_KEY);
    localStorage.removeItem(LEGACY_PATCH_CUSTOM_URL_KEY);
  } catch { }
}

function refreshPatchTargetDefaultText() {
  const defaultTextEl = document.getElementById('patchTargetDefaultText');
  if (defaultTextEl) {
    defaultTextEl.textContent = getDefaultPatchTargetUrl();
  }
}

function getPatchTargetUrl() {
  const stored = normalizePatchTargetUrl(localStorage.getItem(PATCH_TARGET_URL_KEY) || '');
  return stored || getDefaultPatchTargetUrl();
}

function openPatchTargetModal() {
  migrateLegacyPatchTargetConfig();
  refreshPatchTargetDefaultText();
  const modal = document.getElementById('patchTargetModal');
  const input = document.getElementById('patchTargetUrlInput');
  if (!modal || !input) return;
  input.value = normalizePatchTargetUrl(localStorage.getItem(PATCH_TARGET_URL_KEY) || '');
  modal.style.display = 'flex';
  input.focus();
}

function closePatchTargetModal() {
  const modal = document.getElementById('patchTargetModal');
  if (modal) modal.style.display = 'none';
}

function savePatchTargetSetting() {
  const input = document.getElementById('patchTargetUrlInput');
  if (!input) return;
  const normalized = normalizePatchTargetUrl(input.value);
  if (normalized) {
    localStorage.setItem(PATCH_TARGET_URL_KEY, normalized);
    showToast(uiText('补丁目标已保存', 'Patch target saved'), 'success');
  } else {
    localStorage.removeItem(PATCH_TARGET_URL_KEY);
    showToast(uiText('已恢复默认补丁目标', 'Patch target reset to default'), 'success');
  }
  closePatchTargetModal();
  refreshPatchTargetDefaultText();
}

function resetPatchTargetToDefault() {
  localStorage.removeItem(PATCH_TARGET_URL_KEY);
  const input = document.getElementById('patchTargetUrlInput');
  if (input) input.value = '';
  showToast(uiText('已恢复默认补丁目标', 'Patch target reset to default'), 'info');
  refreshPatchTargetDefaultText();
}

migrateLegacyPatchTargetConfig();

// ==================== Routing Strategy ====================

function updateQuotaThresholdVisibility(strategy) {
  const thresholdItem = document.getElementById('quotaThresholdSetting');
  if (!thresholdItem) return;
  thresholdItem.style.display = strategy === 'round-robin' ? 'none' : '';
}

async function setRoutingStrategy(strategy) {
  try {
    const result = await invoke('set_routing_strategy', { strategy });
    // Update button active state
    document.querySelectorAll('[data-routing]').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.routing === result);
    });
    updateQuotaThresholdVisibility(result);
    showToast(result === 'round-robin' ? uiText('已切换为轮询模式', 'Switched to round-robin mode') : uiText('已切换为填充优先模式', 'Switched to fill-first mode'), 'success');
  } catch (e) {
    showToast(uiText('设置失败: ', 'Setting failed: ') + e, 'error');
  }
}

async function restoreRoutingStrategy() {
  try {
    const strategy = await invoke('get_routing_strategy');
    document.querySelectorAll('[data-routing]').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.routing === strategy);
    });
    updateQuotaThresholdVisibility(strategy);
  } catch (e) {
    console.error('Failed to restore routing strategy:', e);
  }
}

// ==================== Official LS ====================

async function onOfficialLsToggle(enabled) {
  try {
    const saved = await invoke('set_official_ls_enabled', { enabled: !!enabled });
    const toggle = document.getElementById('officialLsToggle');
    if (toggle) toggle.checked = !!saved;
    if (saved) {
      await ensureOfficialLsRunningFromSelectedAccount();
    } else {
      try {
        await invoke('stop_official_ls');
      } catch (stopErr) {
        console.warn('Failed to stop official LS after disabling:', stopErr);
      }
    }
    showToast(saved
      ? uiText('已开启官方 LS 转发', 'Official LS forwarding enabled')
      : uiText('已关闭官方 LS 转发', 'Official LS forwarding disabled'),
      'success'
    );
    await refreshOfficialLsStatusUI();
  } catch (e) {
    showToast(uiText('设置 LS 转发失败: ', 'Failed to set LS forwarding: ') + e, 'error');
  }
}

async function restoreOfficialLsEnabled() {
  try {
    const enabled = await invoke('get_official_ls_enabled');
    const toggle = document.getElementById('officialLsToggle');
    if (toggle) toggle.checked = !!enabled;
  } catch (e) {
    console.error('Failed to restore LS enabled state:', e);
  }
}

function getBestAccountForOfficialLsStart() {
  if (!Array.isArray(state.accounts) || state.accounts.length === 0) return null;
  const current = (state.currentIdx >= 0 && state.currentIdx < state.accounts.length)
    ? state.accounts[state.currentIdx]
    : null;
  const hasToken = (acc) => !!(acc && !acc.disabled && String(acc.refresh_token || '').trim());
  if (hasToken(current)) return current;
  return state.accounts.find(hasToken) || null;
}

async function ensureOfficialLsRunningFromSelectedAccount() {
  if (!window.__TAURI__) return false;
  const enabled = await invoke('get_official_ls_enabled');
  if (!enabled) return false;

  const status = await invoke('get_official_ls_status');
  if (status && status.running) return true;

  const account = getBestAccountForOfficialLsStart();
  if (!account) {
    addLog(uiText('官方 LS 启动跳过：没有可用账号', 'Official LS start skipped: no available account'), 'warning');
    return false;
  }

  const accessToken = String(account.access_token || '').trim();
  const refreshToken = String(account.refresh_token || '').trim();
  const expiry = Number(account.expiry_timestamp || 0) || 0;

  if (!refreshToken) {
    addLog(uiText(`官方 LS 启动跳过：账号缺少 refresh_token [${account.email}]`, `Official LS start skipped: missing refresh_token [${account.email}]`), 'warning');
    return false;
  }

  try {
    await invoke('start_official_ls', { accessToken, refreshToken, expiry });
    await refreshOfficialLsStatusUI();
    return true;
  } catch (e) {
    addLog(uiText(`官方 LS 启动失败 [${account.email}]`, `Official LS start failed [${account.email}]`), 'warning', String(e));
    await refreshOfficialLsStatusUI();
    return false;
  }
}

async function refreshOfficialLsStatusUI() {
  try {
    const status = await invoke('get_official_ls_status');
    renderOfficialLsStatus(status);
  } catch (e) {
    renderOfficialLsStatus(null, String(e));
  }
}

function renderOfficialLsStatus(status, overrideText = '') {
  const statusEl = document.getElementById('officialLsStatus');
  const binaryEl = document.getElementById('officialLsBinaryInfo');

  if (statusEl) {
    if (overrideText) {
      statusEl.textContent = uiText(`LS 状态：${overrideText}`, `LS Status: ${overrideText}`);
      statusEl.style.color = '#ef4444';
    } else if (!status) {
      statusEl.textContent = uiText('LS 状态：未获取', 'LS Status: Not available');
      statusEl.style.color = '#9ca3af';
    } else if (status.running) {
      const port = status.https_port ? `:${status.https_port}` : '';
      const pid = status.pid ? ` PID=${status.pid}` : '';
      statusEl.textContent = uiText(
        `LS 状态：运行中${port}${pid}`,
        `LS Status: Running${port}${pid}`
      );
      statusEl.style.color = '#22c55e';
    } else {
      const err = status.last_error ? ` (${status.last_error})` : '';
      statusEl.textContent = uiText(
        `LS 状态：未运行${err}`,
        `LS Status: Not running${err}`
      );
      statusEl.style.color = status.last_error ? '#ef4444' : '#9ca3af';
    }
  }

  if (binaryEl) {
    if (status && status.binary_path) {
      binaryEl.textContent = uiText(
        `二进制文件：${status.binary_path}`,
        `Binary: ${status.binary_path}`
      );
      binaryEl.style.color = '#22c55e';
    } else {
      binaryEl.textContent = uiText('二进制文件：未找到', 'Binary: Not found');
      binaryEl.style.color = '#f59e0b';
    }
  }
}

function normalizeHttpProtocolMode(mode) {
  const normalized = String(mode || '').trim().toLowerCase();
  if (normalized === 'http10' || normalized === 'h10' || normalized === 'http1.0' || normalized === '1.0') return 'http10';
  if (normalized === 'http1' || normalized === 'h1' || normalized === 'http1.1') return 'http1';
  if (normalized === 'http2' || normalized === 'h2') return 'http2';
  return 'auto';
}

function formatHttpProtocolModeLabel(mode) {
  const normalized = normalizeHttpProtocolMode(mode);
  if (normalized === 'http10') return 'HTTP/1.0';
  if (normalized === 'http1') return 'HTTP/1.1';
  if (normalized === 'http2') return 'HTTP/2';
  return uiText('自动协商', 'Auto');
}

function updateHttpProtocolModeUI(mode) {
  const normalized = normalizeHttpProtocolMode(mode);
  document.querySelectorAll('[data-http-protocol]').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.httpProtocol === normalized);
  });
}

async function setHttpProtocolMode(mode) {
  try {
    const saved = await invoke('set_http_protocol_mode', { mode });
    const normalized = normalizeHttpProtocolMode(saved);
    updateHttpProtocolModeUI(normalized);
    showToast(uiText(`上游协议已设置为 ${formatHttpProtocolModeLabel(normalized)}`, `Upstream protocol set to ${formatHttpProtocolModeLabel(normalized)}`), 'success');
  } catch (e) {
    showToast(uiText('设置上游协议失败: ', 'Failed to set upstream protocol: ') + e, 'error');
    await restoreHttpProtocolMode();
  }
}

async function restoreHttpProtocolMode() {
  try {
    const mode = await invoke('get_http_protocol_mode');
    updateHttpProtocolModeUI(mode);
  } catch (e) {
    console.error('Failed to restore HTTP protocol setting:', e);
    updateHttpProtocolModeUI('auto');
  }
}

async function onCapacityFailoverChange(enabled) {
  try {
    const saved = await invoke('set_capacity_failover_enabled', { enabled: !!enabled });
    const toggle = document.getElementById('capacityFailoverToggle');
    if (toggle) toggle.checked = !!saved;
    showToast(saved ? uiText('已开启模型互补', 'Model failover enabled') : uiText('已关闭模型互补', 'Model failover disabled'), 'success');
  } catch (e) {
    showToast(uiText('设置容量自动重试失败: ', 'Capacity failover setting failed: ') + e, 'error');
    await restoreCapacityFailover();
  }
}

async function restoreCapacityFailover() {
  try {
    const enabled = await invoke('get_capacity_failover_enabled');
    const toggle = document.getElementById('capacityFailoverToggle');
    if (toggle) toggle.checked = !!enabled;
  } catch (e) {
    console.error('Failed to restore capacity failover setting:', e);
  }
}



function updateUpstreamServerUI(server) {
  const select = document.getElementById('upstreamServer');
  if (!select) return;

  const normalized = ['custom', 'sandbox'].includes(server) ? server : 'sandbox';
  select.dataset.value = normalized;
  const option = select.querySelector(`.custom-select-option[data-value="${normalized}"]`);
  const display = select.querySelector('.custom-select-display span');
  if (option && display) {
    display.textContent = option.textContent.trim();
  }
  select.querySelectorAll('.custom-select-option').forEach(o => {
    o.classList.toggle('active', o.dataset.value === normalized);
  });
  const panel = document.getElementById('upstreamCustomPanel');
  if (panel) panel.style.display = normalized === 'custom' ? '' : 'none';
}

async function setUpstreamServerConfig(server, displayText = '') {
  try {
    const saved = await invoke('set_upstream_server', { server });
    updateUpstreamServerUI(saved);
    if (displayText) localStorage.setItem('ag-upstream-display', displayText);
    localStorage.setItem('ag-upstream-server', saved);
    if (saved !== 'custom') {
      showToast(uiText('已启用固定回退：sandbox -> daily -> prod', 'Fixed fallback enabled: sandbox -> daily -> prod'), 'success');
    }
  } catch (e) {
    showToast(uiText('保存上游服务器失败: ', 'Failed to save upstream server: ') + e, 'error');
  }
}

async function saveUpstreamCustomUrlConfig() {
  const el = document.getElementById('upstreamCustomUrl');
  if (!el) return;
  const customUrl = (el.value || '').trim();
  try {
    const saved = await invoke('set_upstream_custom_url', { customUrl });
    if (saved !== customUrl) {
      el.value = saved;
    }
    localStorage.setItem('ag-upstream-custom-url', saved);
  } catch (e) {
    showToast(uiText('保存自定义上游地址失败: ', 'Failed to save custom upstream URL: ') + e, 'error');
  }
}

async function restoreUpstreamServerConfig() {
  let server = 'sandbox';
  try {
    server = await invoke('get_upstream_server');
  } catch (e) {
    console.error('Failed to restore upstream server, falling back to local cache:', e);
    server = localStorage.getItem('ag-upstream-server') || 'sandbox';
  }

  updateUpstreamServerUI(server);

  if (server === 'custom') {
    try {
      const savedCustomUrl = await invoke('get_upstream_custom_url');
      const input = document.getElementById('upstreamCustomUrl');
      if (input) input.value = savedCustomUrl || '';
      localStorage.setItem('ag-upstream-custom-url', savedCustomUrl || '');
    } catch (e) {
      const input = document.getElementById('upstreamCustomUrl');
      const fallback = localStorage.getItem('ag-upstream-custom-url') || '';
      if (input) input.value = fallback;
      console.error('Failed to restore custom upstream URL:', e);
    }
  }
}



async function applyPatch() {
  try {
    const targetUrl = getPatchTargetUrl();
    if (!targetUrl) {
      showToast(uiText('请输入目标 URL', 'Please enter target URL'), 'warning');
      return;
    }
    showToast(uiText(`正在应用 IDE 补丁 (目标: ${targetUrl})...`, `Applying IDE patch (target: ${targetUrl})...`), 'info');
    const result = await invoke('apply_patch', { targetUrl });
    showToast(result || uiText('补丁应用成功', 'Patch applied successfully'), 'success');
    await checkPatchStatus();
  } catch (e) {
    showToast(uiText('补丁失败: ', 'Patch failed: ') + e, 'error');
  }
}

async function removePatch() {
  try {
    showToast(uiText('正在撤销 IDE 补丁...', 'Removing IDE patch...'), 'info');
    const result = await invoke('remove_patch');
    showToast(result || uiText('补丁已撤销', 'Patch removed'), 'success');
    await checkPatchStatus();
  } catch (e) {
    showToast(uiText('撤销失败: ', 'Removal failed: ') + e, 'error');
  }
}

async function checkPatchStatus() {
  try {
    const status = await invoke('check_patch_status');
    const el = document.getElementById('patchStatus');
    if (status.applied) {
      el.innerHTML = `<span class="status-dot online"></span><span>${status.message || uiText('补丁已应用', 'Patch applied')}</span>`;
      state.patchApplied = true;
    } else {
      el.innerHTML = `<span class="status-dot offline"></span><span>${status.message || uiText('未应用补丁', 'Patch not applied')}</span>`;
      state.patchApplied = false;
    }
  } catch (e) {
    console.error('Failed to check patch status:', e);
  }
}

// ==================== Certificate Management ====================

async function importCert() {
  try {
    showToast(uiText('正在导入证书 (需要管理员权限)...', 'Importing certificate (admin required)...'), 'info');
    const result = await invoke('import_cert');
    showToast(result || uiText('证书导入成功', 'Certificate imported'), 'success');
    await checkCertStatus();
  } catch (e) {
    showToast(uiText('证书导入失败: ', 'Certificate import failed: ') + e, 'error');
  }
}

async function removeCert() {
  try {
    showToast(uiText('正在卸载证书...', 'Removing certificate...'), 'info');
    const result = await invoke('remove_cert');
    showToast(result || uiText('证书已卸载', 'Certificate removed'), 'success');
    await checkCertStatus();
  } catch (e) {
    showToast(uiText('证书卸载失败: ', 'Certificate removal failed: ') + e, 'error');
  }
}

async function checkCertStatus() {
  try {
    const status = await invoke('check_cert_status');
    const el = document.getElementById('certStatus');
    if (status.installed) {
      el.innerHTML = `<span class="status-dot online"></span><span>${uiText('证书已安装', 'Certificate installed')}</span>`;
      state.certInstalled = true;
    } else {
      el.innerHTML = `<span class="status-dot offline"></span><span>${uiText('证书未安装', 'Certificate not installed')}</span>`;
      state.certInstalled = false;
    }
  } catch (e) {
    console.error('Failed to check cert status:', e);
  }
}

// ==================== Credential Management ====================

async function restoreCurrentAccountSelection(accounts) {
  // Prefer email to restore selection; avoids index mismatch on reorder
  const savedEmail = localStorage.getItem(CURRENT_ACCOUNT_EMAIL_KEY);
  const savedIndexRaw = localStorage.getItem(CURRENT_ACCOUNT_INDEX_KEY);
  let restoredIdx = -1;

  if (savedEmail) {
    restoredIdx = accounts.findIndex(a => a.email === savedEmail);
  }
  if (restoredIdx < 0 && savedIndexRaw !== null) {
    const parsed = parseInt(savedIndexRaw, 10);
    if (!Number.isNaN(parsed) && parsed >= 0 && parsed < accounts.length) {
      restoredIdx = parsed;
    }
  }
  if (restoredIdx >= 0) {
    state.currentIdx = restoredIdx;
  } else if (accounts.length > 0) {
    state.currentIdx = 0;
  } else {
    state.currentIdx = -1;
  }

  // Sync restored account index to backend on startup
  if (window.__TAURI__ && state.currentIdx >= 0) {
    try {
      await invoke('switch_account', { index: state.currentIdx });
    } catch (e) {
      console.warn('Failed to sync current account to backend:', e);
    }
  }
}

async function loadCredentials() {
  if (credentialsLoading) return;
  credentialsLoading = true;
  const runId = ++credentialsLoadRunId;

  try {
    // Show empty state first, then progressively fill in
    state.accounts = [];
    state.currentIdx = -1;
    streamingAccountsUiMode = true;
    renderAccounts();
    updateDashboard();

    if (window.__TAURI__) {
      const seenEmails = new Set();
      const streamRunId = Date.now() + Math.floor(Math.random() * 1000);

      try {
        await new Promise(async (resolve, reject) => {
          let unlisten = null;
          let finished = false;
          let flushTimer = null;
          let flushWarmupTimer = null;
          let streamDone = false;
          const pendingAccounts = [];

          const cleanup = () => {
            if (typeof unlisten === 'function') {
              unlisten();
            }
            if (flushTimer) {
              clearInterval(flushTimer);
              flushTimer = null;
            }
            if (flushWarmupTimer) {
              clearTimeout(flushWarmupTimer);
              flushWarmupTimer = null;
            }
          };

          const tryFinish = () => {
            if (finished) return;
            if (!streamDone) return;
            if (pendingAccounts.length > 0) return;
            finished = true;
            cleanup();
            resolve();
          };

          const ensureFlush = () => {
            if (flushTimer || flushWarmupTimer) return;
            flushWarmupTimer = setTimeout(() => {
              flushWarmupTimer = null;
              flushTimer = setInterval(() => {
                if (runId !== credentialsLoadRunId) {
                  cleanup();
                  return;
                }
                const next = pendingAccounts.shift();
                if (next) {
                  const insertIdx = state.accounts.length;
                  state.accounts.push(next);
                  const searchQuery = (document.getElementById('accountSearch')?.value || '').trim();
                  if (streamingAccountsUiMode && !searchQuery) {
                    appendLoadedAccountCard(next, insertIdx);
                  } else {
                    renderAccounts();
                  }
                  updateAccountLoadProgressUI();
                }
                tryFinish();
              }, ACCOUNT_LOAD_UI_INTERVAL_MS);
            }, ACCOUNT_LOAD_UI_WARMUP_MS);
          };

          try {
            unlisten = await listen('accounts-load-progress', (event) => {
              if (runId !== credentialsLoadRunId) return;
              const payload = event?.payload || {};
              if (payload?.run_id !== streamRunId) return;

              const account = payload?.account;
              if (account && account.email && !seenEmails.has(account.email)) {
                seenEmails.add(account.email);
                pendingAccounts.push(account);
                ensureFlush();
              }

              if (payload?.done && !finished) {
                streamDone = true;
                if (pendingAccounts.length > 0) ensureFlush();
                tryFinish();
              }
            });

            await invoke('load_credentials_stream', { runId: streamRunId });
          } catch (e) {
            cleanup();
            reject(e);
          }
        });
      } catch (streamErr) {
        console.warn('Streaming load failed, falling back to normal load:', streamErr);
        const accounts = await invoke('load_credentials');
        state.accounts = accounts;
        renderAccounts();
        updateDashboard();
      }

      if (runId === credentialsLoadRunId) {
        await restoreCurrentAccountSelection(state.accounts);
        persistCurrentAccountSelection();
        streamingAccountsUiMode = false;
        renderAccounts();
        updateDashboard();
        return;
      } else {
        streamingAccountsUiMode = false;
        return;
      }
    }

    const accounts = await invoke('load_credentials');
    state.accounts = accounts;
    await restoreCurrentAccountSelection(accounts);
    persistCurrentAccountSelection();
    streamingAccountsUiMode = false;
    renderAccounts();
    updateDashboard();
  } catch (e) {
    // Empty directory is not an error
    state.accounts = [];
    state.currentIdx = -1;
    persistCurrentAccountSelection();
    streamingAccountsUiMode = false;
    renderAccounts();
    updateDashboard();
  } finally {
    if (runId === credentialsLoadRunId) {
      credentialsLoading = false;
    }
  }
}

// ==================== Account Import (Dropdown Menu) ====================

function toggleImportMenu() {
  const menu = document.getElementById('importMenu');
  menu.classList.toggle('show');
}

// Close menu when clicking outside
document.addEventListener('click', (e) => {
  const dropdown = document.getElementById('importDropdown');
  if (dropdown && !dropdown.contains(e.target)) {
    document.getElementById('importMenu')?.classList.remove('show');
  }
});

// Method 1: Import credential files from folder
async function importFromFiles() {
  document.getElementById('importMenu')?.classList.remove('show');
  try {
    showToast(uiText('请选择凭证文件...', 'Please select credential files...'), 'info');
    const count = await invoke('import_credential_files');
    if (count > 0) {
      await loadCredentials();
      showToast(uiText(`成功导入 ${count} 个账号`, `Imported ${count} account(s)`), 'success');
    } else {
      showToast(uiText('未导入新账号（可能已存在或取消选择）', 'No new accounts imported (may already exist or cancelled)'), 'info');
    }
  } catch (e) {
    showToast(uiText('导入失败: ', 'Import failed: ') + e, 'error');
  }
}

// Method 2: Paste Refresh Token
function showRefreshTokenDialog() {
  document.getElementById('importMenu')?.classList.remove('show');
  const rt = prompt(uiText('请粘贴 Google Refresh Token:', 'Paste your Google Refresh Token:'));
  if (rt && rt.trim()) {
    importRefreshToken(rt.trim());
  }
}

async function importRefreshToken(rt) {
  try {
    showToast(uiText('正在验证 Refresh Token...', 'Verifying Refresh Token...'), 'info');
    const email = await invoke('import_refresh_token', { refreshToken: rt });
    await loadCredentials();
    showToast(uiText(`账号 ${email} 导入成功！`, `Account ${email} imported!`), 'success');
  } catch (e) {
    showToast(uiText('导入失败: ', 'Import failed: ') + e, 'error');
  }
}

// Method 3: Google OAuth login
async function startOAuthLogin() {
  document.getElementById('importMenu')?.classList.remove('show');
  try {
    showToast(uiText('正在启动 Google 登录...', 'Starting Google login...'), 'info');
    const email = await invoke('start_oauth_login');
    await loadCredentials();
    showToast(uiText(`账号 ${email} 登录成功！`, `Account ${email} logged in!`), 'success');
  } catch (e) {
    showToast(uiText('OAuth 登录失败: ', 'OAuth login failed: ') + e, 'error');
  }
}

function escapeHtml(value) {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/\"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function formatQuotaErrorTitle(quotaError) {
  if (!quotaError || !quotaError.message) return '';
  const codeText = quotaError.code ? ` [${quotaError.code}]` : '';
  return `${uiText('额度异常', 'Quota error')}${codeText}: ${quotaError.message}`;
}

function buildAccountCardHtml(account, originalIdx) {
  const isActive = originalIdx === state.currentIdx;
  const quotaData = state.quotas[account.email];
  const isDisabled = !!account.disabled;
  const isForbidden = !!quotaData?.is_forbidden;
  const hasQuotaError = !!account.quota_error?.message;
  const disabledTitle = account.disabled_reason
    ? uiText(`已禁用: ${account.disabled_reason}`, `Disabled: ${account.disabled_reason}`)
    : uiText('已禁用（refresh_token 可能失效）', 'Disabled (refresh_token may have expired)');
  const quotaErrorTitle = formatQuotaErrorTitle(account.quota_error);
  const quotaGridHtml = renderQuotaGrid(quotaData);
  const emailSafe = escapeHtml(account.email || '');
  const accountInitial = escapeHtml((((account.email || '').trim().charAt(0)) || '?').toUpperCase());

  return `
    <div class="account-card ${isActive ? 'active' : ''} ${isDisabled ? 'disabled' : ''}">
      <div class="account-card-row">
        <div class="account-card-left">
          <div class="account-card-identity">
            <span class="account-card-initial">${accountInitial}</span>
            <div class="account-card-email" title="${emailSafe}">${emailSafe}</div>
          </div>
          <div class="account-card-badges">
            ${isActive ? `<span class="badge current">${uiText('当前', 'Active')}</span>` : ''}
            ${isDisabled ? `<span class="badge disabled" title="${escapeHtml(disabledTitle)}">${uiText('禁用', 'Disabled')}</span>` : ''}
            ${isForbidden ? `<span class="badge forbidden" title="${uiText('额度接口返回 403，账号无权限', 'Quota API returned 403, access denied')}">403</span>` : ''}
            ${hasQuotaError ? `<span class="badge error" title="${escapeHtml(quotaErrorTitle)}">${uiText('异常', 'Error')}</span>` : ''}
          </div>
          <div class="account-card-actions">
            <button class="card-btn card-btn-switch" onclick="switchAccount(${originalIdx})" title="${uiText('切换到此账号', 'Switch to this account')}">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M8 3l4 4-4 4"/><path d="M12 7H3"/><path d="M16 21l-4-4 4-4"/><path d="M12 17h9"/></svg>
            </button>
            <button class="card-btn card-btn-quota" onclick="fetchSingleQuota(${originalIdx})" title="${uiText('刷新额度', 'Refresh quota')}">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
            </button>
            <button class="card-btn ${isDisabled ? 'card-btn-switch' : 'card-btn-delete'}" onclick="toggleAccountDisabled(${originalIdx}, ${!isDisabled})" title="${isDisabled ? uiText('启用账号', 'Enable account') : uiText('禁用账号', 'Disable account')}">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">${isDisabled ? '<path d="M18.36 6.64a9 9 0 1 1-12.73 0"/><line x1="12" y1="2" x2="12" y2="12"/>' : '<circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/>'}</svg>
            </button>
            <button class="card-btn card-btn-delete" onclick="deleteAccount(${originalIdx})" title="${uiText('移除账号', 'Remove account')}">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
            </button>
          </div>
        </div>
        <div class="account-card-right">
          ${quotaGridHtml}
        </div>
      </div>
    </div>
  `;
}

async function syncAccountStateFromBackend() {
  if (!window.__TAURI__) return;
  try {
    const currentEmail = state.currentIdx >= 0 ? state.accounts[state.currentIdx]?.email : null;
    const accounts = await invoke('load_credentials');
    state.accounts = accounts;
    if (currentEmail) {
      const idx = state.accounts.findIndex((a) => a.email === currentEmail);
      state.currentIdx = idx >= 0 ? idx : (state.accounts.length > 0 ? 0 : -1);
    } else {
      state.currentIdx = state.accounts.length > 0 ? 0 : -1;
    }
    persistCurrentAccountSelection();
  } catch (e) {
    console.warn('Failed to sync account state:', e);
  }
}

function renderAccounts() {
  const grid = document.getElementById('accountGrid');
  const searchQuery = (document.getElementById('accountSearch')?.value || '').toLowerCase();

  const matchedIndices = [];
  state.accounts.forEach((account, originalIdx) => {
    if (!searchQuery || account.email.toLowerCase().includes(searchQuery)) {
      matchedIndices.push(originalIdx);
    }
  });

  if (matchedIndices.length === 0) {
    grid.innerHTML = `
            <div class="empty-state">
                <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#555" stroke-width="1.5"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" y1="8" x2="19" y2="14"/><line x1="22" y1="11" x2="16" y2="11"/></svg>
                <p>${searchQuery ? '没有匹配的账号' : '暂无账号'}</p>
                <span>${searchQuery ? '请尝试其他搜索词' : '点击「添加账号」导入'}</span>
            </div>
        `;
    return;
  }

  grid.innerHTML = matchedIndices
    .map((originalIdx) => buildAccountCardHtml(state.accounts[originalIdx], originalIdx))
    .join('');
}

function appendLoadedAccountCard(account, originalIdx) {
  const grid = document.getElementById('accountGrid');
  if (!grid) return;
  const empty = grid.querySelector('.empty-state');
  if (empty) grid.innerHTML = '';
  grid.insertAdjacentHTML('beforeend', buildAccountCardHtml(account, originalIdx));
}

function filterAccounts() {
  renderAccounts();
}

async function switchAccount(idx) {
  try {
    await invoke('switch_account', { index: idx });
    state.currentIdx = idx;
    persistCurrentAccountSelection();
    renderAccounts();
    updateDashboard();
    showToast(uiText(`已切换到 ${state.accounts[idx]?.email}`, `Switched to ${state.accounts[idx]?.email}`), 'success');
  } catch (e) {
    showToast(uiText('切换失败: ', 'Switch failed: ') + e, 'error');
  }
}

async function deleteAccount(idx) {
  if (!confirm(uiText(`确定要移除 ${state.accounts[idx]?.email} 吗？`, `Remove ${state.accounts[idx]?.email}?`))) return;
  try {
    await invoke('delete_account', { index: idx });
    state.accounts.splice(idx, 1);
    if (state.accounts.length === 0) {
      state.currentIdx = -1;
    } else if (idx < state.currentIdx) {
      state.currentIdx -= 1;
    } else if (state.currentIdx >= state.accounts.length) {
      state.currentIdx = state.accounts.length - 1;
    }
    if (window.__TAURI__ && state.currentIdx >= 0) {
      try {
        await invoke('switch_account', { index: state.currentIdx });
      } catch (e) {
        console.warn('Failed to sync current account after deletion:', e);
      }
    }
    persistCurrentAccountSelection();
    renderAccounts();
    updateDashboard();
    showToast(uiText('账号已移除', 'Account removed'), 'success');
  } catch (e) {
    showToast(uiText('移除失败: ', 'Remove failed: ') + e, 'error');
  }
}

async function toggleAccountDisabled(idx, disabled) {
  try {
    const accounts = await invoke('toggle_account_disabled', { index: idx, disabled });
    state.accounts = accounts;
    renderAccounts();
    updateDashboard();
    const email = accounts[idx]?.email || '';
    showToast(disabled ? uiText(`已禁用 ${email}`, `Disabled ${email}`) : uiText(`已启用 ${email}`, `Enabled ${email}`), 'success');
  } catch (e) {
    showToast(uiText('操作失败: ', 'Operation failed: ') + e, 'error');
  }
}

// ==================== Quota Query ====================

// Quota data cache: { email: QuotaData } persisted to localStorage
try {
  const cached = localStorage.getItem('ag_quota_cache');
  state.quotas = cached ? JSON.parse(cached) : {};
} catch { state.quotas = {}; }

function saveQuotasCache() {
  try {
    localStorage.setItem('ag_quota_cache', JSON.stringify(state.quotas));
  } catch { }
}

function getQuotaColor(percentage) {
  if (percentage >= 80) return '#22c55e';  // 绿色
  if (percentage >= 50) return '#14b8a6';  // 青色
  if (percentage >= 20) return '#f59e0b';  // 黄色
  return '#ef4444';                        // 红色
}

function formatResetTime(resetTimeStr) {
  if (!resetTimeStr) return '';
  try {
    const reset = new Date(resetTimeStr);
    const now = new Date();
    const diffMs = reset - now;
    if (diffMs <= 0) return '已重置';
    const hours = Math.floor(diffMs / 3600000);
    const mins = Math.floor((diffMs % 3600000) / 60000);
    if (hours > 0) return `${hours}h${mins}m`;
    return `${mins}m`;
  } catch {
    return '';
  }
}

function renderQuotaGrid(quotaData) {
  // Define 4 fixed quota display slots
  const slots = [
    { key: 'Claude', label: 'Claude', icon: '🟠', fixedColor: '#f59e0b' },
    { key: 'Gemini Image', label: 'Image', icon: '🖼️' },
    { key: 'Gemini Pro', label: 'Pro', icon: '🔵' },
    { key: 'Gemini Flash', label: 'Flash', icon: '⚡' },
  ];

  if (quotaData && quotaData.is_forbidden) {
    return `<div class="quota-forbidden">🚫 ${uiText('账号被禁 (403)', 'Account forbidden (403)')}</div>`;
  }

  // Build model lookup map
  const modelMap = {};
  if (quotaData && quotaData.models) {
    for (const m of quotaData.models) {
      modelMap[m.name] = m;
    }
  }

  return '<div class="quota-grid">' +
    slots.map(slot => {
      const m = modelMap[slot.key];
      if (!m) {
        return `
          <div class="quota-cell quota-cell-empty">
            <div class="quota-cell-head">
              <span class="quota-cell-label"><span class="quota-cell-icon">${slot.icon}</span>${slot.label}</span>
              <span class="quota-cell-value">--</span>
            </div>
            <div class="quota-cell-meter"><span class="quota-cell-bar" style="width:0%;"></span></div>
            <span class="quota-cell-reset quota-cell-reset-empty">${uiText('未查询', 'Not queried')}</span>
          </div>`;
      }
      const color = slot.fixedColor || getQuotaColor(m.percentage);
      const resetText = formatResetTime(m.reset_time);
      return `
        <div class="quota-cell" style="--quota-color:${color};">
          <div class="quota-cell-head">
            <span class="quota-cell-label"><span class="quota-cell-icon">${slot.icon}</span>${slot.label}</span>
            <span class="quota-cell-value">${m.percentage}%</span>
          </div>
          <div class="quota-cell-meter">
            <span class="quota-cell-bar" style="width:${Math.min(m.percentage, 100)}%;"></span>
          </div>
          <span class="quota-cell-reset">${resetText ? uiText(`重置 ${resetText}`, `Reset ${resetText}`) : uiText('重置时间未知', 'Reset time unknown')}</span>
        </div>`;
    }).join('') +
    '</div>';
}

async function fetchAllQuotasUI() {
  const btn = document.getElementById('btnFetchQuotas');
  if (btn) {
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner-sm"></span> ${uiText('查询中...', 'Querying...')}`;
  }

  try {
    showToast(uiText('正在查询所有账号额度...', 'Querying all account quotas...'), 'info');
    const results = await invoke('fetch_all_quotas');

    // results [[email, QuotaData], ...]
    for (const [email, quota] of results) {
      state.quotas[email] = quota;
    }
    saveQuotasCache();
    await syncAccountStateFromBackend();

    renderAccounts();
    showToast(uiText(`额度查询完成: ${results.length} 个账号`, `Quota query complete: ${results.length} accounts`), 'success');
  } catch (e) {
    await syncAccountStateFromBackend();
    showToast(uiText('额度查询失败: ', 'Quota query failed: ') + e, 'error');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.innerHTML = `
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21.21 15.89A10 10 0 1 1 8 2.83" />
          <path d="M22 12A10 10 0 0 0 12 2v10z" />
        </svg>
        查询额度`;
    }
  }
}

async function fetchSingleQuota(idx) {
  try {
    showToast(`正在查询 ${state.accounts[idx]?.email} 的额度...`, 'info');
    const quota = await invoke('fetch_quota', { index: idx });
    const email = state.accounts[idx]?.email;
    if (email) {
      state.quotas[email] = quota;
      saveQuotasCache();
    }
    await syncAccountStateFromBackend();
    renderAccounts();
    showToast(`${email} 额度查询完成`, 'success');
  } catch (e) {
    const account = state.accounts[idx];
    if (account) {
      account.quota_error = {
        code: null,
        message: String(e),
        timestamp: Math.floor(Date.now() / 1000),
      };
    }
    await syncAccountStateFromBackend();
    renderAccounts();
    showToast(uiText('额度查询失败: ', 'Quota query failed: ') + e, 'error');
  }
}

// ==================== Proxy Control ====================

function getThresholdDescription(value) {
  const descriptions = {
    0: uiText('仅额度用尽时切换（0%）', 'Switch only when quota depleted (0%)'),
    20: uiText('额度低于 20% 提前切换', 'Switch early when quota below 20%'),
    40: uiText('额度低于 40% 提前切换', 'Switch early when quota below 40%'),
    60: uiText('额度低于 60% 提前切换', 'Switch early when quota below 60%'),
    80: uiText('额度低于 80% 提前切换', 'Switch early when quota below 80%'),
  };
  return descriptions[value] || descriptions[0];
}

async function setQuotaThreshold(threshold) {
  try {
    const result = await invoke('set_quota_threshold', { threshold });
    document.querySelectorAll('[data-threshold]').forEach(btn => {
      btn.classList.toggle('active', parseInt(btn.dataset.threshold, 10) === result);
    });
    const descEl = document.getElementById('thresholdDesc');
    if (descEl) descEl.textContent = getThresholdDescription(result);
  } catch (e) {
    showToast(uiText('设置额度阈值失败: ', 'Failed to set quota threshold: ') + e, 'error');
  }
}

async function restoreQuotaThreshold() {
  try {
    const threshold = await invoke('get_quota_threshold');
    document.querySelectorAll('[data-threshold]').forEach(btn => {
      btn.classList.toggle('active', parseInt(btn.dataset.threshold, 10) === threshold);
    });
    const descEl = document.getElementById('thresholdDesc');
    if (descEl) descEl.textContent = getThresholdDescription(threshold);
  } catch {
    // keep default UI state
  }
}

let autoQuotaRefreshTimer = null;

function onAutoQuotaRefreshChange(enabled) {
  localStorage.setItem('ag-auto-quota-refresh', enabled ? '1' : '0');
  if (enabled) {
    startAutoQuotaRefresh();
  } else {
    stopAutoQuotaRefresh();
  }
}

function startAutoQuotaRefresh() {
  stopAutoQuotaRefresh();
  autoRefreshQuota();
  autoQuotaRefreshTimer = setInterval(autoRefreshQuota, 120000);
}

function stopAutoQuotaRefresh() {
  if (autoQuotaRefreshTimer) {
    clearInterval(autoQuotaRefreshTimer);
    autoQuotaRefreshTimer = null;
  }
}

async function autoRefreshQuota() {
  if (!state.proxyRunning || state.accounts.length === 0) return;
  try {
    const results = await invoke('fetch_all_quotas');
    for (const [email, quota] of results) {
      state.quotas[email] = quota;
    }
    saveQuotasCache();
    renderAccounts();
  } catch (e) {
    console.warn('Auto quota refresh failed:', e);
  }
}

async function onHeaderPassthroughChange(enabled) {
  try {
    const result = await invoke('set_header_passthrough', { enabled });
    const toggle = document.getElementById('headerPassthroughToggle');
    if (toggle) toggle.checked = !!result;
  } catch (e) {
    showToast(uiText('设置透传模式失败: ', 'Failed to set passthrough: ') + e, 'error');
    const toggle = document.getElementById('headerPassthroughToggle');
    if (toggle) toggle.checked = !enabled;
  }
}

async function restoreHeaderPassthrough() {
  try {
    const enabled = await invoke('get_header_passthrough');
    const toggle = document.getElementById('headerPassthroughToggle');
    if (toggle) toggle.checked = !!enabled;
  } catch {
    // keep default checked state from HTML
  }
}

function formatTokenNumber(value) {
  return Number(value || 0).toLocaleString();
}

function formatTokenMillions(value) {
  const num = Number(value || 0);
  if (!Number.isFinite(num) || num <= 0) return '0.00M';
  return `${(num / 1_000_000).toFixed(2)}M`;
}

function normalizeNonNegativeNumber(value) {
  const num = Number(value);
  if (!Number.isFinite(num) || num < 0) return 0;
  return Math.floor(num);
}

function applyTokenStatsToState(stats) {
  const source = stats || {};
  state.tokenStats = {
    total_input: normalizeNonNegativeNumber(source.total_input),
    total_output: normalizeNonNegativeNumber(source.total_output),
    total_cache_read: normalizeNonNegativeNumber(source.total_cache_read),
    total_cache_creation: normalizeNonNegativeNumber(source.total_cache_creation),
    total_tokens: normalizeNonNegativeNumber(source.total_tokens),
    total_requests: normalizeNonNegativeNumber(source.total_requests),
    total_errors: normalizeNonNegativeNumber(source.total_errors),
  };
  state.totalRequests = state.tokenStats.total_requests;
  state.totalErrors = state.tokenStats.total_errors;
}

function getAbnormalAccountCount() {
  return state.accounts.reduce((count, account) => {
    if (!account || !account.email) return count;
    if (account.disabled) return count + 1;
    // Only count auth errors (token revoked), ignore transient rate-limit/upstream errors
    const qe = account.quota_error;
    if (qe && qe.message) {
      const kind = qe.kind || '';
      const isAuthBroken = kind === 'auth_invalid_grant'
        || kind === 'auth_verification_required'
        || (qe.message.includes('Token') && qe.message.includes('Bad Request'));
      if (isAuthBroken) return count + 1;
    }
    return count;
  }, 0);
}

async function refreshDashboardMetrics({ silent = true } = {}) {
  if (dashboardMetricsLoading) return;
  dashboardMetricsLoading = true;
  try {
    const stats = await invoke('get_token_stats');
    applyTokenStatsToState(stats);
    updateDashboard();
  } catch (e) {
    if (!silent) {
      showToast(uiText('刷新仪表盘统计失败: ', 'Failed to refresh dashboard stats: ') + e, 'error');
    }
  } finally {
    dashboardMetricsLoading = false;
  }
}

function queueDashboardMetricsRefresh(delayMs = 500) {
  if (dashboardMetricsRefreshTimer) return;
  dashboardMetricsRefreshTimer = setTimeout(async () => {
    dashboardMetricsRefreshTimer = null;
    await refreshDashboardMetrics({ silent: true });
  }, delayMs);
}

async function loadTokenStats() {
  try {
    const stats = await invoke('get_token_stats');
    applyTokenStatsToState(stats);
    updateDashboard();

    const inputEl = document.getElementById('statsInputTokens');
    const outputEl = document.getElementById('statsOutputTokens');
    const cacheReadEl = document.getElementById('statsCacheReadTokens');
    const totalEl = document.getElementById('statsTotalTokens');
    const reqEl = document.getElementById('statsTotalRequests');

    if (inputEl) inputEl.textContent = formatTokenMillions(state.tokenStats.total_input);
    if (outputEl) outputEl.textContent = formatTokenMillions(state.tokenStats.total_output);
    if (cacheReadEl) cacheReadEl.textContent = formatTokenMillions(state.tokenStats.total_cache_read);
    if (totalEl) totalEl.textContent = formatTokenMillions(state.tokenStats.total_tokens);
    if (reqEl) reqEl.textContent = formatTokenNumber(state.tokenStats.total_requests);

    const tbody = document.getElementById('tokenStatsTableBody');
    if (!tbody) return;

    if (!stats.accounts || stats.accounts.length === 0) {
      tbody.innerHTML = `
        <tr>
          <td colspan="6" style="padding:24px; text-align:center; color: var(--text-secondary);">暂无统计数据</td>
        </tr>
      `;
      return;
    }

    tbody.innerHTML = stats.accounts.map(acc => `
      <tr style="border-bottom: 1px solid var(--border);">
        <td style="padding:10px 12px;"><span style="font-weight:500;">${escapeHtml(acc.email)}</span></td>
        <td style="padding:10px 12px; text-align:right; font-family:monospace; color:#3b82f6;">${formatTokenMillions(acc.total_input)}</td>
        <td style="padding:10px 12px; text-align:right; font-family:monospace; color:#22c55e;">${formatTokenMillions(acc.total_output)}</td>
        <td style="padding:10px 12px; text-align:right; font-family:monospace; color:#a855f7;">${formatTokenMillions(acc.total_cache_read)}</td>
        <td style="padding:10px 12px; text-align:right; font-family:monospace; font-weight:600;">${formatTokenMillions(acc.total_tokens)}</td>
        <td style="padding:10px 12px; text-align:right; font-family:monospace;">${acc.request_count || 0}</td>
      </tr>
    `).join('');
  } catch (e) {
    showToast(uiText('加载 Token 统计失败: ', 'Failed to load token stats: ') + e, 'error');
  }
}

async function resetTokenStats() {
  if (!confirm(uiText('确定要重置全部 Token 统计吗？此操作不可恢复。', 'Reset all token stats? This cannot be undone.'))) return;
  try {
    const result = await invoke('reset_token_stats');
    showToast(result || 'Token 统计已重置', 'success');
    await loadTokenStats();
  } catch (e) {
    showToast(uiText('重置 Token 统计失败: ', 'Failed to reset token stats: ') + e, 'error');
  }
}

async function toggleProxy() {
  try {
    if (state.proxyRunning) {
      await invoke('stop_proxy');
      state.proxyRunning = false;
      showToast(uiText('代理已停止', 'Proxy stopped'), 'info');
    } else {
      await invoke('start_proxy');
      state.proxyRunning = true;
      showToast(uiText('代理已启动', 'Proxy started'), 'success');
    }
    updateDashboard();
    queueDashboardMetricsRefresh(300);
  } catch (e) {
    showToast(uiText('操作失败: ', 'Operation failed: ') + e, 'error');
  }
}

// ==================== Port Configuration ====================

async function savePortConfig() {
  try {
    const proxyPort = parseInt(document.getElementById('proxyPort').value, 10);
    if (Number.isNaN(proxyPort) || proxyPort < 1024 || proxyPort > 65535) {
      showToast(uiText('端口范围必须在 1024-65535 之间', 'Port must be between 1024 and 65535'), 'warning');
      return;
    }
    await invoke('save_port_config', { proxyPort });
    showToast(uiText('端口配置已保存', 'Port config saved'), 'success');
  } catch (e) {
    showToast(uiText('保存失败: ', 'Save failed: ') + e, 'error');
  }
}

// ==================== AI Provider Management ====================

function onPortChange(value) {
  const port = parseInt(value, 10);
  if (Number.isNaN(port) || port < 1024 || port > 65535) {
    showToast(uiText('端口范围必须在 1024-65535 之间', 'Port must be between 1024 and 65535'), 'warning');
    return;
  }

  refreshPatchTargetDefaultText();
  savePortConfig();
}

function showAddProviderForm() {
  document.getElementById('providerFormTitle').textContent = '添加 AI 供应商';
  document.getElementById('providerEditIndex').value = '-1';
  document.getElementById('providerName').value = '';
  document.getElementById('providerBaseUrl').value = '';
  document.getElementById('providerApiKey').value = '';
  document.getElementById('providerApiKey').type = 'password';
  selectProtocol('openai');
  document.getElementById('modelMappingList').innerHTML = '';
  document.getElementById('providerFormOverlay').style.display = 'flex';
}

function showEditProviderForm(idx) {
  const provider = state.providers[idx];
  if (!provider) return;

  document.getElementById('providerFormTitle').textContent = '编辑 AI 供应商';
  document.getElementById('providerEditIndex').value = idx;
  document.getElementById('providerName').value = provider.name || '';
  document.getElementById('providerBaseUrl').value = provider.base_url || '';
  document.getElementById('providerApiKey').value = provider.api_key || '';
  document.getElementById('providerApiKey').type = 'password';
  selectProtocol(provider.protocol || 'openai');

  // Load model mappings
  const mappingList = document.getElementById('modelMappingList');
  mappingList.innerHTML = '';
  if (provider.model_map && Object.keys(provider.model_map).length > 0) {
    for (const [from, to] of Object.entries(provider.model_map)) {
      addModelMappingRow(from, to);
    }
  }

  document.getElementById('providerFormOverlay').style.display = 'flex';
}

function hideProviderForm() {
  document.getElementById('providerFormOverlay').style.display = 'none';
}

function selectProtocol(protocol) {
  document.querySelectorAll('#providerFormOverlay .protocol-btn[data-protocol]').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.protocol === protocol);
  });
}

function getSelectedProtocol() {
  const active = document.querySelector('#providerFormOverlay .protocol-btn[data-protocol].active');
  return active ? active.dataset.protocol : 'openai';
}

function addModelMappingRow(fromVal = '', toVal = '') {
  const list = document.getElementById('modelMappingList');
  const row = document.createElement('div');
  row.className = 'model-mapping-row';
  row.innerHTML = `
    <input type="text" class="mapping-from" placeholder="反重力模型名（如 gemini-2.5-flash）" value="${escapeHtml(fromVal)}" />
    <span class="mapping-arrow">→</span>
    <input type="text" class="mapping-to" placeholder="供应商模型名（如 gpt-4o）" value="${escapeHtml(toVal)}" />
    <button class="mapping-remove-btn" onclick="this.parentElement.remove()" title="移除">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    </button>
  `;
  list.appendChild(row);
}

function toggleApiKeyVisibility() {
  const input = document.getElementById('providerApiKey');
  input.type = input.type === 'password' ? 'text' : 'password';
}

function collectModelMappings() {
  const map = {};
  document.querySelectorAll('.model-mapping-row').forEach(row => {
    const from = row.querySelector('.mapping-from').value.trim();
    const to = row.querySelector('.mapping-to').value.trim();
    if (from && to) {
      map[from] = to;
    }
  });
  return map;
}

async function saveProvider() {
  const name = document.getElementById('providerName').value.trim();
  const baseUrl = document.getElementById('providerBaseUrl').value.trim();
  const apiKey = document.getElementById('providerApiKey').value.trim();
  const protocol = getSelectedProtocol();
  const modelMap = collectModelMappings();
  const editIdx = parseInt(document.getElementById('providerEditIndex').value);

  if (!name) {
    showToast(uiText('请输入供应商名称', 'Please enter provider name'), 'warning');
    return;
  }
  if (!baseUrl) {
    showToast(uiText('请输入 Base URL', 'Please enter Base URL'), 'warning');
    return;
  }

  const provider = {
    name,
    base_url: baseUrl,
    api_key: apiKey,
    protocol,
    model_map: modelMap,
    enabled: true,
  };

  if (editIdx >= 0 && editIdx < state.providers.length) {
    // Preserve original enabled state
    provider.enabled = state.providers[editIdx].enabled;
    state.providers[editIdx] = provider;
  } else {
    state.providers.push(provider);
  }

  renderProviders();
  hideProviderForm();
  persistProviders();
  showToast(`供应商 "${name}" 已保存`, 'success');
}

async function deleteProvider(idx) {
  const provider = state.providers[idx];
  if (!provider) return;
  if (!confirm(uiText(`确定要移除供应商 "${provider.name}" 吗？`, `Remove provider "${provider.name}"?`))) return;

  state.providers.splice(idx, 1);
  renderProviders();
  persistProviders();
  showToast(uiText('供应商已移除', 'Provider removed'), 'success');
}

function toggleProviderEnabled(idx) {
  if (!state.providers[idx]) return;
  state.providers[idx].enabled = !state.providers[idx].enabled;
  renderProviders();
  persistProviders();
}

function renderProviders() {
  const list = document.getElementById('providerList');

  if (state.providers.length === 0) {
    list.innerHTML = `
      <div class="empty-state">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#555" stroke-width="1.5">
          <path d="M12 2L2 7l10 5 10-5-10-5z" />
          <path d="M2 17l10 5 10-5" />
          <path d="M2 12l10 5 10-5" />
        </svg>
        <p>暂无 AI 供应商</p>
        <span>点击「添加供应商」配置第三方 API 供应商</span>
      </div>
    `;
    return;
  }

  const protocolLabels = { openai: 'OpenAI', gemini: 'Gemini', claude: 'Claude' };

  list.innerHTML = state.providers.map((p, i) => {
    const mappingCount = p.model_map ? Object.keys(p.model_map).length : 0;
    const maskedKey = p.api_key ? p.api_key.substring(0, 6) + ' ... ' + p.api_key.slice(-4) : uiText('未设置', 'Not set');

    return `
      <div class="provider-card ${p.enabled ? '' : 'disabled'}">
        <div class="provider-card-header">
          <div class="provider-card-left">
            <div class="provider-card-info">
              <div class="provider-card-name">${escapeHtml(p.name)}</div>
              <div class="provider-card-url" title="${escapeHtml(p.base_url)}">${escapeHtml(p.base_url)}</div>
            </div>
          </div>
          <div class="provider-card-right">
            <span class="provider-badge protocol">${protocolLabels[p.protocol] || p.protocol}</span>
            ${mappingCount > 0 ? `<span class="provider-badge mapping">${uiText(`${mappingCount} 个映射`, `${mappingCount} mappings`)}</span>` : ''}
            <span class="provider-badge ${p.enabled ? 'enabled' : 'disabled-badge'}">${p.enabled ? uiText('已启用', 'Enabled') : uiText('已禁用', 'Disabled')}</span>
          </div>
        </div>
        <div class="provider-card-meta">
          <svg class="provider-key-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/>
          </svg>
          <span class="provider-key-display">${escapeHtml(maskedKey)}</span>
        </div>
        ${mappingCount > 0 ? `
          <div class="provider-card-mappings">
            ${Object.entries(p.model_map).map(([from, to]) => `
              <div class="mapping-tag">
                <span class="mapping-tag-from">${escapeHtml(from)}</span>
                <span class="mapping-tag-arrow">→</span>
                <span class="mapping-tag-to">${escapeHtml(to)}</span>
              </div>
            `).join('')}
          </div>
        ` : ''}
        <div class="provider-card-actions">
          <button class="card-action-btn switch" onclick="toggleProviderEnabled(${i})" title="${p.enabled ? uiText('禁用', 'Disable') : uiText('启用', 'Enable')}">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              ${p.enabled
        ? '<path d="M18.36 6.64a9 9 0 1 1-12.73 0" /><line x1="12" y1="2" x2="12" y2="12" />'
        : '<circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" />'}
            </svg>
          </button>
          <button class="card-action-btn refresh" onclick="showEditProviderForm(${i})" title="${uiText('编辑', 'Edit')}">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
            </svg>
          </button>
          <button class="card-action-btn delete" onclick="deleteProvider(${i})" title="${uiText('移除', 'Remove')}">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="3 6 5 6 21 6" />
              <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
          </button>
        </div>
      </div>
    `;
  }).join('');
}

// Persist providers to localStorage and backend
function persistProviders() {
  const normalized = normalizeProviders(state.providers);
  state.providers = normalized;
  localStorage.setItem('ag-proxy-providers', JSON.stringify(normalized));
  // Push to backend if Tauri is available
  if (window.__TAURI__) {
    invoke('save_providers', { providers: JSON.stringify(normalized) }).catch(e => {
      console.warn('Failed to save providers to backend:', e);
    });
  }
}

function normalizeProvider(raw) {
  const modelMapRaw = raw?.model_map ?? raw?.modelMap ?? {};
  const modelMap = {};
  if (modelMapRaw && typeof modelMapRaw === 'object' && !Array.isArray(modelMapRaw)) {
    Object.entries(modelMapRaw).forEach(([k, v]) => {
      const from = String(k || '').trim();
      const to = String(v || '').trim();
      if (from && to) modelMap[from] = to;
    });
  }
  return {
    name: String(raw?.name || '').trim(),
    base_url: String(raw?.base_url ?? raw?.baseUrl ?? '').trim(),
    api_key: String(raw?.api_key ?? raw?.apiKey ?? '').trim(),
    protocol: String(raw?.protocol || 'openai').trim() || 'openai',
    model_map: modelMap,
    enabled: raw?.enabled !== false,
  };
}

function normalizeProviders(list) {
  if (!Array.isArray(list)) return [];
  return list
    .map(normalizeProvider)
    .filter(p => p.name && p.base_url);
}

function loadProviders() {
  if (window.__TAURI__) {
    invoke('load_saved_providers').then(data => {
      try {
        state.providers = normalizeProviders(JSON.parse(data));
        renderProviders();
      } catch (e) {
        console.error('Failed to parse provider config:', e);
        loadProvidersFromLocalStorage();
      }
    }).catch(e => {
      console.warn('Failed to load providers from backend, falling back to localStorage:', e);
      loadProvidersFromLocalStorage();
    });
  } else {
    loadProvidersFromLocalStorage();
  }
}

function loadProvidersFromLocalStorage() {
  try {
    const saved = localStorage.getItem('ag-proxy-providers');
    if (saved) {
      state.providers = normalizeProviders(JSON.parse(saved));
      renderProviders();
    } else {
      state.providers = [];
      renderProviders();
    }
  } catch (e) {
    console.error('Failed to load provider config:', e);
    state.providers = [];
    renderProviders();
  }
}

function persistCurrentAccountSelection() {
  try {
    if (state.currentIdx >= 0 && state.accounts[state.currentIdx]) {
      localStorage.setItem(CURRENT_ACCOUNT_EMAIL_KEY, state.accounts[state.currentIdx].email || '');
      localStorage.setItem(CURRENT_ACCOUNT_INDEX_KEY, String(state.currentIdx));
    } else {
      localStorage.removeItem(CURRENT_ACCOUNT_EMAIL_KEY);
      localStorage.removeItem(CURRENT_ACCOUNT_INDEX_KEY);
    }
  } catch { }
}

// ==================== Request Flow Tracing ====================

const FLOW_MAX_ENTRIES = 200;
let flowEntries = [];

function getStatusClass(status) {
  if (!status) return 'sErr';
  if (status >= 200 && status < 300) return 's2xx';
  if (status >= 400 && status < 500) return 's4xx';
  return 's5xx';
}

function isSuccessStatus(status) {
  const code = Number(status);
  return Number.isFinite(code) && code >= 200 && code < 300;
}

function buildFlowHopStates(hops) {
  const states = [];
  let blocked = false;

  for (const hop of hops) {
    const code = Number(hop?.status);
    const hasStatus = Number.isFinite(code);

    if (blocked) {
      states.push('unreached');
      continue;
    }

    if (!hasStatus) {
      states.push('disconnected');
      blocked = true;
      continue;
    }

    if (code >= 200 && code < 300) {
      states.push('success');
    } else {
      states.push('failed');
      blocked = true;
    }
  }

  return states;
}

function getNodeSvg(name) {
  const svgs = {
    '客户端': '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>',
    '本地代理': '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>',
    '网关': '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
    'LS桥接': '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
    '上游官方': '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/></svg>',
  };
  return svgs[name] || '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2"/></svg>';
}

function getNodeIconClass(state) {
  if (state === 'success') return 'active';
  if (state === 'failed' || state === 'disconnected') return 'error';
  return 'inactive';
}

function renderFlowChain(hops, direction) {
  let html = '';
  const hopStates = buildFlowHopStates(hops);

  for (let i = 0; i < hops.length; i++) {
    const hop = hops[i];
    const state = hopStates[i] || 'unreached';
    const isLast = i === hops.length - 1;
    const iconClass = getNodeIconClass(state);
    const nodeIconHtml = (state === 'disconnected' || state === 'failed')
      ? '<span class="flow-node-fail">×</span>'
      : getNodeSvg(hop.node);

    html += `<div class="flow-node">
      <div class="flow-node-icon ${iconClass}">${nodeIconHtml}</div>
      <div class="flow-node-label">${escapeHtml(translateBackendMsg(hop.node))}</div>
    </div>`;

    if (!isLast) {
      const nextState = hopStates[i + 1] || 'unreached';
      if (nextState === 'unreached') {
        continue;
      }

      const nextHop = hops[i + 1];
      const arrowStatus = direction === 'forward' ? nextHop.status : hop.status;
      const arrowShowsDisconnected = direction === 'forward'
        ? nextState === 'disconnected'
        : state === 'disconnected';
      const arrowShowsX = arrowShowsDisconnected || !isSuccessStatus(arrowStatus);
      const statusText = arrowShowsX ? '×' : Number(arrowStatus);
      const statusCls = arrowShowsX ? 'sErr' : getStatusClass(Number(arrowStatus));
      const arrowSvg = direction === 'forward'
        ? '<svg width="40" height="12" viewBox="0 0 40 12"><line x1="0" y1="6" x2="34" y2="6" stroke="currentColor" stroke-width="1.5"/><polyline points="30,2 36,6 30,10" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>'
        : '<svg width="40" height="12" viewBox="0 0 40 12"><line x1="6" y1="6" x2="40" y2="6" stroke="currentColor" stroke-width="1.5"/><polyline points="10,2 4,6 10,10" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>';
      const detailHopIndex = direction === 'forward' ? (i + 1) : i;

      html += `<div class="flow-arrow">
        <div class="flow-arrow-status ${statusCls} flow-status-chip" data-flow-dir="${direction}" data-flow-hop-index="${detailHopIndex}" title="点击查看该链路详情" onclick="openFlowDetailFromStatus(this, event)">${statusText}</div>
        <div class="flow-arrow-line ${direction}">${arrowSvg}</div>
      </div>`;
    }
  }

  return html;
}

function renderFlowEntry(flow) {
  const isSuccess = flow.final_status && flow.final_status >= 200 && flow.final_status < 300;
  const statusCls = getStatusClass(flow.final_status);
  const entryClass = isSuccess ? 'is-success' : 'is-error';

  // Shorten path for display
  const pathShort = flow.path.length > 60 ? flow.path.substring(0, 57) + '...' : flow.path;

  const entry = document.createElement('div');
  entry.className = `flow-entry ${entryClass}`;
  entry.dataset.flowId = flow.id;

  entry.innerHTML = `
    <div class="flow-entry-header" onclick="toggleFlowEntry(this)">
      <span class="flow-entry-time">${escapeHtml(flow.timestamp)}</span>
      <span class="flow-entry-method">${escapeHtml(flow.method)}</span>
      <span class="flow-entry-path" title="${escapeHtml(flow.path)}">${escapeHtml(pathShort)}</span>
      <span class="flow-entry-account" title="${escapeHtml(flow.account)}">${escapeHtml(flow.account)}</span>
      <span class="flow-entry-status ${statusCls} flow-status-chip" data-flow-dir="summary" title="点击查看该请求总结" onclick="openFlowDetailFromStatus(this, event)">${flow.final_status || 'ERR'}</span>
      <span class="flow-entry-elapsed">${flow.elapsed_ms}ms</span>
      <svg class="flow-entry-toggle" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="9 18 15 12 9 6" />
      </svg>
    </div>
    <div class="flow-entry-body">
      <div class="flow-chain-section">
        <div class="flow-chain-label">📤 请求方向 (→)</div>
        <div class="flow-chain">
          ${renderFlowChain(flow.forward_hops, 'forward')}
        </div>
      </div>
      <div class="flow-chain-section">
        <div class="flow-chain-label">📥 响应方向 (←)</div>
        <div class="flow-chain">
          ${renderFlowChain(flow.return_hops, 'return')}
        </div>
      </div>
      <div class="flow-entry-detail-label">链路详情（点击状态码切换）</div>
      <pre class="flow-entry-detail">${formatFlowDetail(flow.detail || '点击状态码查看该链路详情')}</pre>
    </div>
  `;

  return entry;
}

function toggleFlowEntry(headerEl) {
  const entry = headerEl.closest('.flow-entry');
  if (entry) {
    entry.classList.toggle('expanded');
  }
}

function formatFlowDetail(rawDetail) {
  return escapeHtml(rawDetail || '');
}

function getFlowById(flowId) {
  if (!flowId) return null;
  return flowEntries.find((item) => item && item.id === flowId) || null;
}

function resolveFlowDetailByChip(flow, chipEl) {
  if (!flow || !chipEl) return '';
  const flowDir = chipEl.dataset.flowDir || 'summary';
  if (flowDir === 'summary') {
    return flow.detail || '';
  }
  const hopIndex = Number(chipEl.dataset.flowHopIndex);
  const hops = flowDir === 'return' ? flow.return_hops : flow.forward_hops;
  if (!Array.isArray(hops) || !Number.isInteger(hopIndex) || hopIndex < 0 || hopIndex >= hops.length) {
    return flow.detail || '';
  }
  const hopDetail = hops[hopIndex]?.detail;
  if (typeof hopDetail === 'string' && hopDetail.trim()) {
    return hopDetail;
  }
  return flow.detail || '';
}

function openFlowDetailFromStatus(el, event) {
  if (event) {
    event.preventDefault();
    event.stopPropagation();
  }
  const entry = el?.closest('.flow-entry');
  if (!entry) return;
  const flow = getFlowById(entry.dataset.flowId);
  const detail = resolveFlowDetailByChip(flow, el) || '暂无该链路详情';
  entry.classList.add('expanded');
  const detailEl = entry.querySelector('.flow-entry-detail');
  if (detailEl) {
    detailEl.textContent = detail;
    detailEl.scrollIntoView({ block: 'nearest' });
  }
}

function addFlowEntry(flow) {
  const container = document.getElementById('flowEntriesInner');
  if (!container) return;

  // Remove empty state if exists
  const emptyState = container.querySelector('.flow-empty-state');
  if (emptyState) emptyState.remove();

  flowEntries.unshift(flow);
  const entryEl = renderFlowEntry(flow);

  // Insert at the top
  if (container.firstChild) {
    container.insertBefore(entryEl, container.firstChild);
  } else {
    container.appendChild(entryEl);
  }

  // Limit entries
  while (flowEntries.length > FLOW_MAX_ENTRIES) {
    flowEntries.pop();
    if (container.lastChild) container.removeChild(container.lastChild);
  }
}

function clearFlowEntries() {
  flowEntries = [];
  const container = document.getElementById('flowEntriesInner');
  if (container) {
    container.innerHTML = `
      <div class="flow-empty-state">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" style="color: var(--text-dim);">
          <path d="M5 12h14" />
          <path d="M12 5l7 7-7 7" />
        </svg>
        <p>暂无请求记录</p>
        <span>启动代理后，请求的完整链路追踪将显示在这里</span>
      </div>
    `;
  }
}



async function initFlowListener() {
  if (window.__TAURI__) {
    try {
      await listen('request-flow', (event) => {
        const flow = event.payload;
        if (flow && flow.id) {
          addFlowEntry(flow);
        }
      });
      console.log('Request flow listener registered');
    } catch (e) {
      console.error('Failed to register request-flow listener:', e);
    }
  }
}

// ==================== Logging ====================

function addLog(msg, type = '', details = null) {
  msg = translateBackendMsg(msg);
  const logArea = document.getElementById('logArea');
  const ts = new Date().toLocaleTimeString();
  if (!logArea) {
    if (details) {
      console.log(`[${ts}] ${msg}`, details);
    } else {
      console.log(`[${ts}] ${msg}`);
    }
    return;
  }
  const line = document.createElement('div');
  line.className = `log-line ${type} ${details ? 'has-details' : ''}`;

  const header = document.createElement('div');
  header.className = 'log-header';
  header.textContent = `[${ts}] ${msg}`;
  if (details) {
    header.innerHTML += ` <span class="details-toggle">${uiText('(点击查看详情)', '(click for details)')}</span>`;
    line.onclick = () => {
      const detailEl = line.querySelector('.log-details');
      const isVisible = detailEl.style.display === 'block';
      detailEl.style.display = isVisible ? 'none' : 'block';
      line.classList.toggle('expanded', !isVisible);
    };
  }
  line.appendChild(header);

  if (details) {
    const detailEl = document.createElement('pre');
    detailEl.className = 'log-details';
    try {
      // Try to pretty-print JSON
      const parsed = JSON.parse(details);
      detailEl.textContent = JSON.stringify(parsed, null, 2);
    } catch (e) {
      detailEl.textContent = details;
    }
    detailEl.style.display = 'none';
    line.appendChild(detailEl);
  }

  logArea.appendChild(line);

  // If in error-only mode, hide non-error lines
  if (currentLogMode === 'error' && type !== 'error' && type !== 'warning') {
    line.style.display = 'none';
  }

  logArea.scrollTop = logArea.scrollHeight;

  if (type === 'success' || type === 'error') {
    queueDashboardMetricsRefresh(type === 'error' ? 120 : 600);
  }

  // Limit log line count
  while (logArea.children.length > 500) {
    logArea.removeChild(logArea.firstChild);
  }
}

function clearLogs() {
  const logArea = document.getElementById('logArea');
  if (!logArea) return;
  logArea.innerHTML = '<div class="log-line dim">[系统] 日志已清空</div>';
}

let currentLogMode = 'all'; // 'all' | 'error' | 'flow'

function setLogMode(mode) {
  currentLogMode = mode;
  // Update tab buttons
  document.querySelectorAll('.log-mode-btn').forEach(btn => btn.classList.remove('active'));
  const btn = document.getElementById(mode === 'all' ? 'logModeAll' : mode === 'error' ? 'logModeError' : 'logModeFlow');
  if (btn) btn.classList.add('active');

  const logViewContainer = document.getElementById('logViewContainer');
  const flowContainer = document.getElementById('flowEntriesContainer');

  if (mode === 'flow') {
    if (logViewContainer) logViewContainer.style.display = 'none';
    if (flowContainer) flowContainer.style.display = '';
  } else {
    if (logViewContainer) logViewContainer.style.display = '';
    if (flowContainer) flowContainer.style.display = 'none';
  }

  // Apply error filter
  if (mode === 'error') {
    const logArea = document.getElementById('logArea');
    if (logArea) {
      Array.from(logArea.children).forEach(line => {
        const isError = line.classList.contains('error') || line.classList.contains('warning');
        line.style.display = isError ? '' : 'none';
      });
    }
  } else if (mode === 'all') {
    const logArea = document.getElementById('logArea');
    if (logArea) {
      Array.from(logArea.children).forEach(line => {
        line.style.display = '';
      });
    }
  }
}

function clearCurrentLogView() {
  if (currentLogMode === 'flow') {
    clearFlowEntries();
  } else {
    clearLogs();
  }
}

// ==================== Dashboard Update ====================

function updateDashboard() {
  const proxyStatusText = state.proxyRunning ? uiText('运行中', 'Running') : uiText('未启动', 'Stopped');
  const proxyStatusColor = state.proxyRunning ? '#22c55e' : '#ef4444';
  const abnormalCount = getAbnormalAccountCount();

  if (document.getElementById('dashProxyStatus')) {
    document.getElementById('dashProxyStatus').textContent = proxyStatusText;
    document.getElementById('dashProxyStatus').style.color = proxyStatusColor;
  }

  if (document.getElementById('dashAccountCount')) {
    document.getElementById('dashAccountCount').textContent = formatTokenNumber(state.accounts.length);
  }
  if (document.getElementById('dashAbnormalAccountCount')) {
    document.getElementById('dashAbnormalAccountCount').textContent = formatTokenNumber(abnormalCount);
  }
  if (document.getElementById('dashTotalRequests')) {
    document.getElementById('dashTotalRequests').textContent = formatTokenNumber(state.totalRequests);
  }
  if (document.getElementById('dashTotalErrors')) {
    document.getElementById('dashTotalErrors').textContent = formatTokenNumber(state.totalErrors);
  }
  if (document.getElementById('dashInputTokens')) {
    document.getElementById('dashInputTokens').textContent = formatTokenMillions(state.tokenStats.total_input);
  }
  if (document.getElementById('dashOutputTokens')) {
    document.getElementById('dashOutputTokens').textContent = formatTokenMillions(state.tokenStats.total_output);
  }


  // Sidebar status indicator
  const indicator = document.getElementById('proxyStatusIndicator');
  if (indicator) {
    if (state.proxyRunning) {
      indicator.innerHTML = `<span class="status-dot online"></span><span class="status-text">${uiText('代理运行中', 'Proxy running')}</span>`;
    } else {
      indicator.innerHTML = `<span class="status-dot offline"></span><span class="status-text">${uiText('代理未启动', 'Proxy stopped')}</span>`;
    }
  }

  // Current account card
  const card = document.getElementById('currentAccountCard');
  if (card) {
    if (state.currentIdx >= 0 && state.accounts[state.currentIdx]) {
      const acct = state.accounts[state.currentIdx];
      const initial = acct.email ? acct.email.charAt(0).toUpperCase() : '?';
      card.innerHTML = `
              <div class="account-avatar">${initial}</div>
              <div class="account-info">
                  <span class="account-email">${escapeHtml(acct.email)}</span>
                  <span class="account-meta">${uiText('使用中', 'In use')}</span>
              </div>
          `;
    } else {
      card.innerHTML = `
              <div class="account-avatar">?</div>
              <div class="account-info">
                  <span class="account-email">${uiText('未选择账号', 'No account selected')}</span>
                  <span class="account-meta">${uiText('请前往账号管理选择', 'Choose one in Accounts')}</span>
              </div>
          `;
    }
  }


  // Update start/stop button
  const proxyBtn = document.getElementById('proxyToggleBtn');
  if (proxyBtn) {
    if (state.proxyRunning) {
      proxyBtn.innerHTML = `
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
                <span>${uiText('停止代理', 'Stop Proxy')}</span>
            `;
      proxyBtn.className = 'action-btn danger';
    } else {
      proxyBtn.innerHTML = `
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="5 3 19 12 5 21 5 3"/></svg>
                <span>${uiText('启动代理', 'Start Proxy')}</span>
            `;
      proxyBtn.className = 'action-btn primary';
    }
  }
}

// ==================== Mock (Development Mode) ====================

async function mockInvoke(cmd, args) {
  console.log('[Mock]', cmd, args);

  switch (cmd) {
    case 'load_credentials':
      return [
        { email: 'test-account-1@gmail.com', project: 'project-123' },
        { email: 'test-account-2@gmail.com', project: 'project-456' },
        { email: 'demo-user@gmail.com', project: 'demo-project' },
      ];
    case 'check_patch_status':
      return { applied: false };
    case 'check_cert_status':
      return { installed: false };
    case 'apply_patch':
      return '补丁应用成功';
    case 'remove_patch':
      return '补丁已撤销';
    case 'switch_account':
      return 'ok';
    case 'save_port_config':
      return 'ok';
    case 'set_quota_threshold':
      return Math.max(0, Math.min(80, Number(args?.threshold ?? 0)));
    case 'get_quota_threshold':
      return 0;
    case 'set_header_passthrough':
      return !!args?.enabled;
    case 'get_header_passthrough':
      return true;
    case 'set_transport_mode': {
      const mode = args?.mode === 'client_gateway' ? 'client_gateway' : 'legacy';
      localStorage.setItem('ag-mock-transport-mode', mode);
      return mode;
    }
    case 'get_transport_mode':
      return localStorage.getItem('ag-mock-transport-mode') || 'legacy';
    case 'set_http_protocol_mode': {
      const mode = String(args?.mode || '').toLowerCase();
      const normalized = ['auto', 'http10', 'http1', 'http2'].includes(mode) ? mode : 'auto';
      localStorage.setItem('ag-mock-http-protocol-mode', normalized);
      return normalized;
    }
    case 'get_http_protocol_mode':
      return localStorage.getItem('ag-mock-http-protocol-mode') || 'auto';
    case 'set_capacity_failover_enabled': {
      const enabled = !!args?.enabled;
      localStorage.setItem('ag-mock-capacity-failover-enabled', enabled ? '1' : '0');
      return enabled;
    }
    case 'get_capacity_failover_enabled':
      return localStorage.getItem('ag-mock-capacity-failover-enabled') !== '0';
    case 'set_upstream_server': {
      const server = String(args?.server || '').toLowerCase();
      return server === 'custom' ? 'custom' : 'sandbox';
    }
    case 'get_upstream_server':
      return localStorage.getItem('ag-upstream-server') || 'sandbox';
    case 'set_upstream_custom_url':
      return String(args?.customUrl || '').trim();
    case 'get_upstream_custom_url':
      return localStorage.getItem('ag-upstream-custom-url') || '';
    case 'set_official_ls_enabled': {
      const enabled = !!args?.enabled;
      localStorage.setItem('ag-mock-official-ls-enabled', enabled ? '1' : '0');
      return enabled;
    }
    case 'get_official_ls_enabled':
      return localStorage.getItem('ag-mock-official-ls-enabled') !== '0';
    case 'get_official_ls_status':
      return { running: false, pid: null, binary_path: '', https_port: null, last_error: null };
    case 'start_official_ls':
      return 'Official LS started';
    case 'stop_official_ls':
      return 'Official LS stopped';
    case 'check_official_ls_binary':
      return { available: false, path: '' };
    case 'get_token_stats':
      return {
        total_input: 0,
        total_output: 0,
        total_cache_read: 0,
        total_cache_creation: 0,
        total_tokens: 0,
        total_requests: 0,
        total_errors: 0,
        accounts: [],
      };
    case 'reset_token_stats':
      return 'Token stats reset';
    case 'fetch_all_quotas':
      return [];
    case 'toggle_account_disabled':
      return [];
    case 'flush_token_stats':
      return 'ok';
    default:
      return null;
  }
}

function updateAccountLoadProgressUI() {
  updateDashboard();
}

// ==================== Custom Dropdown ====================

function toggleSelect(displayEl) {
  const select = displayEl.parentElement;
  // Close other open dropdowns
  document.querySelectorAll('.custom-select.open').forEach(el => {
    if (el !== select) el.classList.remove('open');
  });
  select.classList.toggle('open');
}

// Handle option click
document.addEventListener('click', function (e) {
  const option = e.target.closest('.custom-select-option');
  if (option) {
    const select = option.closest('.custom-select');
    const display = select.querySelector('.custom-select-display span');
    // Update display text
    display.textContent = option.textContent;
    select.dataset.value = option.dataset.value;
    // Update active state
    select.querySelectorAll('.custom-select-option').forEach(o => o.classList.remove('active'));
    option.classList.add('active');
    // Close dropdown
    select.classList.remove('open');

    // Show/hide upstream server custom URL panel
    if (select.id === 'upstreamServer') {
      const value = option.dataset.value;
      const text = option.textContent.trim();
      updateUpstreamServerUI(value);
      setUpstreamServerConfig(value, text);
    }

    return;
  }
  // Close when clicking outside
  if (!e.target.closest('.custom-select')) {
    document.querySelectorAll('.custom-select.open').forEach(el => el.classList.remove('open'));
  }
});

// ==================== Theme Switching ====================

function setTheme(theme) {
  document.documentElement.setAttribute('data-theme', theme);
  localStorage.setItem('ag-proxy-theme', theme);

  // Highlight the active theme button
  document.querySelectorAll('.theme-btn').forEach(btn => btn.classList.remove('active'));
  const activeBtn = document.getElementById('theme-' + theme);
  if (activeBtn) activeBtn.classList.add('active');
}

function restoreTheme() {
  const saved = localStorage.getItem('ag-proxy-theme') || 'light';
  setTheme(saved);
}

// ==================== Initialization ====================

// Listen for log events pushed from Rust backend
async function initLogs() {
  console.log('Initializing log listener...');
  if (window.__TAURI__) {
    try {
      await listen('log-event', (event) => {
        console.log('Received log event:', event);
        addLog(event.payload.message, event.payload.type || event.payload.log_type, event.payload.details);
      });
      console.log('Log listener registered successfully');
    } catch (e) {

      console.error('Failed to register log listener:', e);
    }
  } else {
    console.warn('Tauri API not found, logs will not be recorded from backend');
  }
}

async function initAccountSwitchListener() {
  if (window.__TAURI__) {
    try {
      await listen('account-switched', (event) => {
        const newIdx = event.payload;
        if (typeof newIdx === 'number' && newIdx >= 0 && newIdx < state.accounts.length) {
          state.currentIdx = newIdx;
          persistCurrentAccountSelection();
          updateDashboard();
          renderAccounts();
        }
      });
    } catch (e) {
      console.error('Failed to register account-switched listener:', e);
    }
  }
}

function deferNonCriticalStartupChecks() {
  const runChecks = () => {
    checkPatchStatus();
    checkCertStatus();
  };

  if (typeof window.requestIdleCallback === 'function') {
    window.requestIdleCallback(runChecks, { timeout: 1500 });
  } else {
    setTimeout(runChecks, 300);
  }
}

function restoreAutoQuotaRefreshSetting() {
  const enabled = localStorage.getItem('ag-auto-quota-refresh') === '1';
  const toggle = document.getElementById('autoQuotaRefresh');
  if (toggle) toggle.checked = enabled;
  if (enabled) startAutoQuotaRefresh();
}

function onAutoStartProxyChange(enabled) {
  localStorage.setItem('ag-auto-start-proxy', enabled ? '1' : '0');
  showToast(enabled ? uiText('已开启启动时自动启动代理', 'Auto-start proxy on launch enabled') : uiText('已关闭启动时自动启动代理', 'Auto-start proxy on launch disabled'), 'success');
}

function restoreAutoStartProxySetting() {
  const enabled = localStorage.getItem('ag-auto-start-proxy') === '1';
  const toggle = document.getElementById('autoStartProxyToggle');
  if (toggle) toggle.checked = enabled;
}

function formatStartupError(error) {
  if (!error) return 'unknown error';
  if (typeof error === 'string') return error;
  if (error?.message) return error.message;
  return String(error);
}

async function runStartupStep(name, step) {
  try {
    await step();
    return true;
  } catch (e) {
    const detail = formatStartupError(e);
    console.error(`[startup] ${name} failed:`, e);
    addLog(uiText(`初始化失败：${name}`, `Initialization failed: ${name}`), 'warning', detail);
    return false;
  }
}

async function bootstrapApp() {
  await runStartupStep('Restore UI language', async () => {
    restoreUiLanguage();
  });
  await runStartupStep('Restore theme', async () => {
    restoreTheme();
  });
  await runStartupStep('Bind settings events', async () => {
    restoreSettings();
  });
  await runStartupStep('Restore upstream server', restoreUpstreamServerConfig);
  await runStartupStep('Restore official LS', restoreOfficialLsEnabled);
  await runStartupStep('Restore HTTP protocol', restoreHttpProtocolMode);
  await runStartupStep('Restore capacity failover', restoreCapacityFailover);
  await runStartupStep('Refresh LS status', refreshOfficialLsStatusUI);
  await runStartupStep('Restore routing strategy', restoreRoutingStrategy);
  await runStartupStep('Restore quota threshold', restoreQuotaThreshold);
  await runStartupStep('Restore header passthrough', restoreHeaderPassthrough);
  restoreAutoQuotaRefreshSetting();
  restoreAutoStartProxySetting();
  addLog(uiText('系统初始化中...', 'System initializing...'), 'dim');
  await runStartupStep('Initialize log listener', initLogs);
  await runStartupStep('Initialize flow listener', initFlowListener);
  await runStartupStep('Initialize account switch listener', initAccountSwitchListener);
  deferNonCriticalStartupChecks();
  await runStartupStep('Refresh dashboard', () => refreshDashboardMetrics({ silent: true }));
  await runStartupStep('Load accounts', loadCredentials);
  await runStartupStep('Ensure official LS running', ensureOfficialLsRunningFromSelectedAccount);
  await runStartupStep('Load providers', async () => {
    loadProviders();
  });

  // Auto-start proxy: wait until accounts are loaded and synced to backend
  if (localStorage.getItem('ag-auto-start-proxy') === '1' && !state.proxyRunning) {
    if (state.accounts.length === 0) {
      addLog(uiText('自动启动代理：无可用账号，跳过自动启动', 'Auto-start proxy: no available accounts, skipping'), 'warning');
    } else {
      // Wait for switch_account invoke to complete (async inside restoreCurrentAccountSelection)
      setTimeout(async () => {
        if (!state.proxyRunning) {
          addLog(uiText('自动启动代理...', 'Auto-starting proxy...'), 'dim');
          await toggleProxy();
        }
      }, 1200);
    }
  }
}

document.addEventListener('DOMContentLoaded', () => {
  bootstrapApp().catch((e) => {
    const detail = formatStartupError(e);
    console.error('[startup] fatal error:', e);
    addLog(uiText('系统初始化失败', 'System initialization failed'), 'error', detail);
    showToast(uiText(`系统初始化失败: ${detail}`, `System initialization failed: ${detail}`), 'error');
  });
});

function restoreSettings() {
  const upstreamUrlEl = document.getElementById('upstreamCustomUrl');
  if (upstreamUrlEl) {
    upstreamUrlEl.addEventListener('change', saveUpstreamCustomUrlConfig);
    upstreamUrlEl.addEventListener('blur', saveUpstreamCustomUrlConfig);
  }

}
