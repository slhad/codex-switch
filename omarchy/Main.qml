import QtQuick
import Quickshell
import Quickshell.Io

// The Rust binary owns OAuth files and usage API calls. This item only starts
// the collector, validates its display-only JSON, and keeps the last good
// snapshot available while a refresh is running.
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
  readonly property string barTextValue: {
    var revision = root.dataRevision
    return root.barText()
  }
  readonly property string barTooltipValue: {
    var revision = root.dataRevision
    var error = root.errorText
    return root.barTooltip()
  }
  readonly property bool updating: updateProcess.running
  readonly property bool switching: switchProcess.running
  readonly property int refreshIntervalSec: Math.max(30, Number(setting("refreshIntervalSec", 900)))
  readonly property string binaryPath: expandPath(String(setting("binaryPath", "codex-switch")))
  readonly property string percentMode: String(setting("percentMode", "used")).toLowerCase() === "remaining"
    ? "remaining" : "used"

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

  function formatDuration(resetAt) {
    if (!resetAt) return ""
    var remaining = new Date(String(resetAt)).getTime() - root.nowMs
    if (!(remaining > 0)) return "now"
    var minutes = Math.floor(remaining / 60000)
    var hours = Math.floor(minutes / 60)
    var days = Math.floor(hours / 24)
    if (days > 0) return days + "d " + (hours % 24) + "h"
    if (hours > 0) return hours + "h " + (minutes % 60) + "m"
    return Math.max(1, minutes) + "m"
  }

  function accountSummary(account) {
    var window = headlineWindow(account)
    if (!window) return account && account.status === "unavailable" ? "unavailable" : "?"
    var percent = formatPercent(percentValue(window))
    var reset = formatDuration(window.resetAt)
    return window.kind + " " + percent + (reset === "" ? "" : " · " + reset)
  }

  function barText() {
    var account = activeAccount
    if (!account) return "󱚣 ?"
    var window = headlineWindow(account)
    if (!window) return "󱚣 ?"
    var reset = formatDuration(window.resetAt)
    return "󱚣 " + formatPercent(percentValue(window))
      + (reset === "" ? "" : " 󰥔 " + reset)
  }

  function barTooltip() {
    if (accounts.length === 0)
      return root.errorText !== "" ? root.errorText : "No Codex or PI accounts found"
    var lines = ["Codex Switch"]
    for (var i = 0; i < accounts.length; i++) {
      var account = accounts[i]
      var marker = account.current === true ? "* " : "- "
      lines.push(marker + String(account.name || "?") + " · "
        + String(account.email || "?") + " · " + accountSummary(account))
    }
    if (root.errorText !== "") lines.push("Refresh: " + root.errorText)
    return lines.join("\n")
  }

  function alarming(account) {
    var window = headlineWindow(account)
    return !!window && Number(window.usedPercent) >= 90
  }

  function switchSource(source) {
    if (!source || source.switchable !== true || switchProcess.running || updateProcess.running)
      return false
    root.actionStatusText = "Switching to " + String(source.profile || "account") + "..."
    root.lastActionError = ""
    switchProcess.command = [
      root.binaryPath,
      "switch",
      String(source.profile || ""),
      "--target",
      String(source.provider || "")
    ]
    switchProcess.running = true
    return true
  }

  function resetCredits(account) {
    var quota = quotaFor(account)
    return quota && quota.resetCredits ? quota.resetCredits : null
  }
}
