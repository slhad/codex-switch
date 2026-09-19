import QtQuick
import Quickshell
import Quickshell.Io

// The Rust binary owns OAuth files, usage API calls, and process checks. This
// item starts the collector, validates its display-only JSON, and keeps the
// last good snapshot available while a refresh is running.
Item {
  id: root
  visible: false

  property var settings: ({})
  property var snapshot: ({ schemaVersion: 1, accounts: [] })
  property string errorText: ""
  property string actionStatusText: ""
  property string lastActionError: ""
  property bool pendingRefresh: false
  property double nowMs: Date.now()
  property int dataRevision: 0

  readonly property var accounts: snapshot && Array.isArray(snapshot.accounts)
    ? snapshot.accounts : []
  readonly property var activeAccount: {
    var revision = root.dataRevision
    return root.findActiveAccount()
  }
  readonly property var barAccount: {
    var revision = root.dataRevision
    return root.findBarAccount()
  }
  readonly property string barTextValue: {
    var revision = root.dataRevision
    return root.barText()
  }
  readonly property string barDetailValue: {
    var revision = root.dataRevision
    return root.barDetail()
  }
  readonly property string barTooltipValue: {
    var revision = root.dataRevision
    var error = root.errorText
    return root.barTooltip()
  }
  readonly property bool updating: updateProcess.running
  readonly property bool switching: switchProcess.running
  readonly property bool desktopSwitchPending: pendingSource !== null
    || desktopCheckProcess.running || desktopConfirmationOpen
  readonly property string desktopConfirmationMessage: pendingSource
    ? "Codex desktop is running. Kill it and switch to \""
      + String(pendingSource.profile || "account") + "\"?" : ""
  property var pendingSource: null
  property bool desktopConfirmationOpen: false
  property string desktopCheckError: ""
  property bool desktopCheckHandled: false
  property bool streamCheckHandled: false
  property bool streamingDetected: false
  property var streamingProcesses: []
  property string streamCheckError: ""
  property string privacySettingError: ""
  readonly property int refreshIntervalSec: Math.max(30, Number(setting("refreshIntervalSec", 900)))
  readonly property string binaryPath: expandPath(String(setting("binaryPath", "codex-switch")))
  readonly property string percentMode: String(setting("percentMode", "used")).toLowerCase() === "remaining"
    ? "remaining" : "used"
  readonly property bool hideEmailsSetting: setting("hideEmails", false) === true
  readonly property bool autoHideEmailsWhenStreaming: setting("autoHideEmailsWhenStreaming", false) === true
  readonly property int streamingCheckIntervalSec: {
    var value = Number(setting("streamingCheckIntervalSec", 10))
    return isFinite(value) ? Math.max(5, Math.min(60, Math.round(value))) : 10
  }
  readonly property bool streamingCheckRunning: streamingCheckProcess.running
  readonly property bool privacySettingRunning: privacySettingProcess.running
  readonly property bool emailsHidden: root.hideEmailsSetting
    || (root.autoHideEmailsWhenStreaming && root.streamingDetected)
  readonly property string emailPrivacyText: {
    var revision = root.dataRevision
    if (root.privacySettingRunning) return "Updating email privacy setting..."
    if (root.privacySettingError !== "")
      return "Privacy setting error: " + root.privacySettingError
    if (root.hideEmailsSetting) return "Emails hidden by plugin setting"
    if (root.autoHideEmailsWhenStreaming && root.streamingDetected)
      return "Emails hidden while streaming"
    return "Emails visible"
  }
  readonly property string streamingStatusText: {
    var revision = root.dataRevision
    if (root.streamCheckError !== "") return "Streaming check: " + root.streamCheckError
    if (root.streamingCheckRunning) return "Checking for OBS..."
    if (root.streamingDetected) {
      var processes = Array.isArray(root.streamingProcesses) ? root.streamingProcesses : []
      return processes.length > 0
        ? "Streaming detected: " + processes.join(", ")
        : "Streaming detected"
    }
    return "No supported streaming app detected"
  }

  Timer {
    interval: root.refreshIntervalSec * 1000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  Timer {
    interval: 30000
    running: true
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  Timer {
    interval: root.streamingCheckIntervalSec * 1000
    running: root.autoHideEmailsWhenStreaming
    repeat: true
    triggeredOnStart: true
    onTriggered: root.checkStreaming()
  }

  Process {
    id: updateProcess
    running: false

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applySnapshot(text)
    }

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: if (text.trim() !== "") root.errorText = text.trim()
    }

    onExited: function(exitCode) {
      if (exitCode !== 0 && root.errorText === "")
        root.errorText = "codex-switch exited with status " + exitCode
      if (root.pendingRefresh) {
        root.pendingRefresh = false
        Qt.callLater(root.refresh)
      }
    }
  }

  Process {
    id: desktopCheckProcess
    running: false

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyDesktopStatus(text)
    }

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: if (text.trim() !== "") root.desktopCheckError = text.trim()
    }

    onExited: function(exitCode) {
      if (exitCode !== 0 && !root.desktopCheckHandled) {
        root.pendingSource = null
        root.desktopConfirmationOpen = false
        root.actionStatusText = ""
        root.errorText = root.desktopCheckError !== ""
          ? root.desktopCheckError
          : "Could not check Codex desktop (status " + exitCode + ")"
      }
    }
  }

  Process {
    id: switchProcess
    running: false

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.lastActionError = text.trim()
    }

    onExited: function(exitCode) {
      if (exitCode === 0) {
        root.actionStatusText = "Account switched"
        root.lastActionError = ""
        root.refresh()
      } else {
        root.actionStatusText = root.lastActionError !== ""
          ? root.lastActionError
          : "Switch failed with status " + exitCode
      }
    }
  }

  Process {
    id: streamingCheckProcess
    running: false

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyStreamingStatus(text)
    }

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.streamCheckError = text.trim()
    }

    onExited: function(exitCode) {
      if (exitCode !== 0 && !root.streamCheckHandled) {
        root.streamCheckError = root.streamCheckError !== ""
          ? root.streamCheckError
          : "codex-switch exited with status " + exitCode
        root.dataRevision++
      }
    }
  }

  Process {
    id: privacySettingProcess
    running: false

    stdout: StdioCollector { waitForEnd: true }

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.privacySettingError = String(text || "").trim()
    }

    onExited: function(exitCode) {
      if (exitCode === 0) {
        root.privacySettingError = ""
      } else if (root.privacySettingError === "") {
        root.privacySettingError = "omarchy bar set exited with status " + exitCode
      }
      root.dataRevision++
    }
  }

  function setting(name, fallback) {
    var value = root.settings ? root.settings[name] : undefined
    return value === undefined || value === null ? fallback : value
  }

  function expandPath(value) {
    var path = String(value || "").trim()
    if (path === "") return "codex-switch"
    if (path === "~") return Quickshell.env("HOME") || path
    if (path.indexOf("~/") === 0)
      return (Quickshell.env("HOME") || "") + path.substring(1)
    if (path.indexOf("$HOME/") === 0)
      return (Quickshell.env("HOME") || "") + path.substring(5)
    return path
  }

  function refresh() {
    if (updateProcess.running) {
      root.pendingRefresh = true
      return
    }
    root.errorText = ""
    updateProcess.command = [root.binaryPath, "omarchy", "print"]
    updateProcess.running = true
  }

  function checkStreaming() {
    if (streamingCheckProcess.running) return false
    root.streamCheckHandled = false
    root.streamCheckError = ""
    streamingCheckProcess.command = [root.binaryPath, "omarchy", "streaming-status"]
    streamingCheckProcess.running = true
    return true
  }

  function setHideEmails(enabled) {
    if (privacySettingProcess.running) return false
    var next = enabled === true
    root.privacySettingError = ""
    privacySettingProcess.command = [
      "omarchy", "bar", "set", "io.github.slhad.codex-switch",
      "hideEmails", next ? "true" : "false", "--json"
    ]
    privacySettingProcess.running = true
    return true
  }

  function toggleHideEmails() {
    return root.setHideEmails(!root.hideEmailsSetting)
  }

  function applySnapshot(content) {
    try {
      var parsed = JSON.parse(String(content || ""))
      if (!parsed || parsed.schemaVersion !== 1 || !Array.isArray(parsed.accounts))
        throw new Error("unsupported usage snapshot")
      root.snapshot = parsed
      root.dataRevision++
      root.errorText = ""
    } catch (error) {
      root.errorText = "Invalid codex-switch usage data: " + error
    }
  }

  function findActiveAccount() {
    for (var i = 0; i < accounts.length; i++) {
      if (accounts[i] && accounts[i].current === true) return accounts[i]
    }
    return accounts.length > 0 ? accounts[0] : null
  }

  function accountMeta(account) {
    if (!account) return ""
    var parts = []
    if (!root.emailsHidden && String(account.email || "") !== "")
      parts.push(String(account.email))
    if (account.current === true) parts.push("current")
    return parts.join(" · ")
  }

  function accountIdentity(account) {
    return root.emailsHidden ? "" : String(account.email || "?")
  }

  function findBarAccount() {
    var hit = root.snapshot ? root.snapshot.lastQuotaHit : null
    if (hit) {
      var provider = String(hit.provider || "").toLowerCase()
      var profile = String(hit.profile || "")
      var email = String(hit.email || "").toLowerCase()

      for (var i = 0; i < accounts.length; i++) {
        var account = accounts[i]
        var sources = account && Array.isArray(account.sources) ? account.sources : []
        for (var j = 0; j < sources.length; j++) {
          var source = sources[j]
          if (provider !== "" && profile !== ""
              && String(source.provider || "").toLowerCase() === provider
              && String(source.profile || "") === profile)
            return account
        }
      }

      if (email !== "") {
        for (var k = 0; k < accounts.length; k++) {
          if (String(accounts[k].email || "").toLowerCase() === email)
            return accounts[k]
        }
      }
    }
    return root.findActiveAccount()
  }

  function quotaFor(account) {
    return account && account.quota ? account.quota : null
  }

  function headlineWindow(account) {
    var quota = quotaFor(account)
    if (!quota) return null

    if (quota.monthly && quota.monthly.usedPercent !== null
        && quota.monthly.usedPercent !== undefined) {
      return {
        kind: "month",
        usedPercent: Number(quota.monthly.usedPercent),
        remainingPercent: quota.monthly.remainingPercent,
        resetAt: quota.monthly.resetAt || ""
      }
    }

    var best = null
    var windows = Array.isArray(quota.windows) ? quota.windows : []
    for (var i = 0; i < windows.length; i++) {
      var window = windows[i]
      if (!window || window.usedPercent === null || window.usedPercent === undefined) continue
      if (!best || Number(window.usedPercent) > Number(best.usedPercent)) best = window
    }
    return best
  }

  function percentValue(window) {
    if (!window) return null
    if (root.percentMode === "remaining" && window.remainingPercent !== null
        && window.remainingPercent !== undefined)
      return Number(window.remainingPercent)
    return window.usedPercent === null || window.usedPercent === undefined
      ? null : Number(window.usedPercent)
  }

  function formatPercent(value) {
    if (value === null || value === undefined || !isFinite(Number(value))) return "?"
    return String(Math.round(Number(value))) + "%"
  }

  function formatModelUsage(value, units) {
    if (value === null || value === undefined || !isFinite(Number(value))) return "?"
    var formatted = Number(value).toFixed(2).replace(/\.?0+$/, "")
    return formatted + (String(units || "").toLowerCase() === "percent" ? " pp" : "")
  }

  function formatTokenCount(value) {
    if (value === null || value === undefined || !isFinite(Number(value))) return "?"
    return String(Math.round(Number(value))).replace(/\B(?=(\d{3})+(?!\d))/g, ",")
  }

  function formatDuration(resetAt) {
    if (!resetAt) return ""
    var resetMs = new Date(String(resetAt)).getTime()
    if (!isFinite(resetMs)) return ""
    var remaining = resetMs - root.nowMs
    if (!(remaining > 0)) return "now"
    var minutes = Math.floor(remaining / 60000)
    var hours = Math.floor(minutes / 60)
    var days = Math.floor(hours / 24)
    if (days > 0) return days + "d " + (hours % 24) + "h"
    if (hours > 0) return hours + "h " + (minutes % 60) + "m"
    return Math.max(1, minutes) + "m"
  }

  function formatExpiry(timestamp) {
    var date = new Date(String(timestamp || ""))
    if (!isFinite(date.getTime())) return "unknown"

    var month = date.getMonth() + 1
    var day = date.getDate()
    var hours = date.getHours()
    var minutes = date.getMinutes()
    return String(date.getFullYear()) + "-"
      + (month < 10 ? "0" : "") + month + "-"
      + (day < 10 ? "0" : "") + day + " "
      + (hours < 10 ? "0" : "") + hours + ":"
      + (minutes < 10 ? "0" : "") + minutes
  }

  function formatFetchAge(timestamp) {
    var fetchedMs = new Date(String(timestamp || "")).getTime()
    if (!isFinite(fetchedMs)) return "unknown"

    var elapsed = Math.max(0, root.nowMs - fetchedMs)
    if (elapsed < 60000) return "just now"

    var minutes = Math.floor(elapsed / 60000)
    var hours = Math.floor(minutes / 60)
    var days = Math.floor(hours / 24)
    if (days > 0) return days + "d " + (hours % 24) + "h ago"
    if (hours > 0) return hours + "h " + (minutes % 60) + "m ago"
    return minutes + "m ago"
  }

  function formatFetchTime(timestamp) {
    var date = new Date(String(timestamp || ""))
    if (!isFinite(date.getTime())) return "unknown"

    var month = date.getMonth() + 1
    var day = date.getDate()
    var hours = date.getHours()
    var minutes = date.getMinutes()
    var seconds = date.getSeconds()
    return String(date.getFullYear()) + "-"
      + (month < 10 ? "0" : "") + month + "-"
      + (day < 10 ? "0" : "") + day + " "
      + (hours < 10 ? "0" : "") + hours + ":"
      + (minutes < 10 ? "0" : "") + minutes + ":"
      + (seconds < 10 ? "0" : "") + seconds
  }

  function fetchLabel(account) {
    if (!account || !account.lastFetchedAt) return "Last API fetch: unavailable"
    return "Last API fetch: " + formatFetchAge(account.lastFetchedAt)
  }

  function quotaWindowsSummary(account, separator) {
    var quota = quotaFor(account)
    if (!quota) return ""

    var summaries = []
    var windows = Array.isArray(quota.windows) ? quota.windows : []
    for (var i = 0; i < windows.length; i++) {
      var window = windows[i]
      if (!window) continue
      var summary = String(window.kind || "window") + " " + formatPercent(percentValue(window))
      var reset = formatDuration(window.resetAt)
      if (reset !== "") summary += " 󰥔 " + reset
      summaries.push(summary)
    }
    return summaries.join(separator || " · ")
  }

  function accountSummary(account) {
    var window = headlineWindow(account)
    if (!window) return account && account.status === "unavailable" ? "unavailable" : "?"

    if (window.kind === "month") {
      var monthlyReset = formatDuration(window.resetAt)
      return "month " + formatPercent(percentValue(window))
        + (monthlyReset === "" ? "" : " · " + monthlyReset)
    }

    var summary = quotaWindowsSummary(account, " · ")
    return summary === "" ? "?" : summary
  }

  function barDetail() {
    var account = barAccount
    if (!account) return "?"
    var window = headlineWindow(account)
    if (!window) return "?"

    if (window.kind === "month") {
      var reset = formatDuration(window.resetAt)
      return formatPercent(percentValue(window))
        + (reset === "" ? "" : " 󰥔 " + reset)
    }

    var summary = quotaWindowsSummary(account, " · ")
    return summary === "" ? "?" : summary
  }

  function barText() {
    return "\uf915 " + root.barDetail()
  }

  function barTooltip() {
    if (accounts.length === 0)
      return root.errorText !== "" ? root.errorText : "No Codex or PI accounts found"
    var lines = ["Codex Switch"]
    for (var i = 0; i < accounts.length; i++) {
      var account = accounts[i]
      var marker = barAccount && account.key === barAccount.key ? "* " : "- "
      var freshness = account.status === "stale" ? " · cached" : ""
      var fetched = account.lastFetchedAt
        ? " · fetched " + formatFetchAge(account.lastFetchedAt)
          + " · " + formatFetchTime(account.lastFetchedAt)
        : " · fetch time unavailable"
      var identity = root.accountIdentity(account)
      var line = marker + String(account.name || "?")
      if (identity !== "") line += " · " + identity
      line += " · " + accountSummary(account) + freshness + fetched
      lines.push(line)
    }
    if (root.errorText !== "") lines.push("Refresh: " + root.errorText)
    return lines.join("\n")
  }

  function alarming(account) {
    var window = headlineWindow(account)
    return !!window && Number(window.usedPercent) >= 90
  }

  function startSwitch(source, killDesktop) {
    if (!source || source.switchable !== true || switchProcess.running || updateProcess.running)
      return false
    root.actionStatusText = "Switching to " + String(source.profile || "account") + "..."
    root.lastActionError = ""
    var command = [
      root.binaryPath,
      "switch",
      String(source.profile || ""),
      "--target",
      String(source.provider || "")
    ]
    if (killDesktop) command.push("--kill")
    switchProcess.command = command
    switchProcess.running = true
    return true
  }

  function switchSource(source) {
    if (!source || source.switchable !== true || switchProcess.running || updateProcess.running
        || desktopCheckProcess.running || root.desktopConfirmationOpen)
      return false

    if (String(source.provider || "").toLowerCase() !== "codex")
      return root.startSwitch(source, false)

    root.pendingSource = source
    root.desktopCheckHandled = false
    root.desktopCheckError = ""
    root.errorText = ""
    root.actionStatusText = "Checking Codex desktop..."
    desktopCheckProcess.command = [root.binaryPath, "omarchy", "desktop-status"]
    desktopCheckProcess.running = true
    return true
  }

  function applyDesktopStatus(content) {
    root.desktopCheckHandled = true
    try {
      var parsed = JSON.parse(String(content || ""))
      if (!parsed || typeof parsed.running !== "boolean")
        throw new Error("invalid desktop status")

      var source = root.pendingSource
      if (!source) return
      if (parsed.running === true) {
        root.actionStatusText = ""
        root.desktopConfirmationOpen = true
        return
      }

      root.pendingSource = null
      if (!root.startSwitch(source, false))
        root.errorText = "Cannot switch while another action is running"
    } catch (error) {
      root.pendingSource = null
      root.desktopConfirmationOpen = false
      root.actionStatusText = ""
      root.errorText = "Could not check Codex desktop: " + error
    }
  }

  function confirmDesktopSwitch() {
    var source = root.pendingSource
    root.pendingSource = null
    root.desktopConfirmationOpen = false
    if (!root.startSwitch(source, true))
      root.errorText = "Cannot switch while another action is running"
  }

  function cancelDesktopSwitch() {
    root.desktopCheckHandled = true
    if (desktopCheckProcess.running) desktopCheckProcess.running = false
    root.desktopConfirmationOpen = false
    root.pendingSource = null
    root.actionStatusText = "Switch cancelled"
  }

  function resetCredits(account) {
    var quota = quotaFor(account)
    return quota && quota.resetCredits ? quota.resetCredits : null
  }

  function applyStreamingStatus(content) {
    root.streamCheckHandled = true
    try {
      var parsed = JSON.parse(String(content || ""))
      if (!parsed || typeof parsed.streaming !== "boolean"
          || !Array.isArray(parsed.processes))
        throw new Error("invalid streaming status")

      var processes = []
      for (var i = 0; i < parsed.processes.length; i++) {
        if (typeof parsed.processes[i] !== "string")
          throw new Error("invalid streaming process name")
        processes.push(parsed.processes[i])
      }
      root.streamingDetected = parsed.streaming
      root.streamingProcesses = processes
      root.streamCheckError = ""
      root.dataRevision++
    } catch (error) {
      root.streamCheckError = String(error)
      root.dataRevision++
    }
  }

  onSettingsChanged: {
    root.dataRevision++
    if (root.autoHideEmailsWhenStreaming) Qt.callLater(root.checkStreaming)
  }
}
