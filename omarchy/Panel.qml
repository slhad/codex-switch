import QtQuick
import QtQuick.Controls
import Quickshell
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.slhad.codex-switch"
  ipcTarget: root.moduleName
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  property string selectedKey: ""
  property bool cursorActive: false

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property color track: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.16)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property var accounts: usage.accounts
  readonly property int selectedIndex: selectedAccountIndex()
  readonly property var selectedAccount: selectedIndex >= 0 && selectedIndex < accounts.length
    ? accounts[selectedIndex] : null
  readonly property var selectedQuota: selectedAccount ? selectedAccount.quota : null
  readonly property string barText: usage.barTextValue
  readonly property string barDetail: usage.barDetailValue
  readonly property string barTooltip: usage.barTooltipValue
  readonly property bool alarming: usage.alarming(usage.barAccount)

  function open() {
    usage.refresh()
    root.controller.show()
    Qt.callLater(function() {
      if (root.opened) keyCatcher.forceActiveFocus()
    })
  }

  function close() {
    if (usage.desktopSwitchPending) usage.cancelDesktopSwitch()
    root.controller.hide()
  }

  function toggle() {
    if (root.opened) root.close()
    else root.open()
  }

  function refresh() {
    usage.refresh()
  }

  function switchPanel(direction) {
    var identity = root.hostWidget || root
    if (root.bar && typeof root.bar.switchPanelFrom === "function")
      return root.bar.switchPanelFrom(identity, direction)
    return false
  }

  function selectedAccountIndex() {
    for (var i = 0; i < accounts.length; i++) {
      if (accounts[i] && accounts[i].key === root.selectedKey) return i
    }
    if (usage.activeAccount) {
      for (var j = 0; j < accounts.length; j++) {
        if (accounts[j] && accounts[j].key === usage.activeAccount.key) return j
      }
    }
    return accounts.length > 0 ? 0 : -1
  }

  function ensureSelection() {
    if (accounts.length === 0) {
      root.selectedKey = ""
      return
    }
    if (selectedAccountIndex() < 0)
      root.selectedKey = usage.activeAccount ? usage.activeAccount.key : accounts[0].key
  }

  function selectAccount(index) {
    if (accounts.length === 0) return
    var wrapped = ((index % accounts.length) + accounts.length) % accounts.length
    root.cursorActive = true
    root.selectedKey = accounts[wrapped].key
  }

  function nextAccount() {
    selectAccount(selectedIndex + 1)
  }

  function percentLabel(window) {
    var value = usage.percentValue(window)
    return usage.formatPercent(value)
  }

  function resetLabel(window) {
    var reset = usage.formatDuration(window ? window.resetAt : "")
    return reset === "" ? "" : "Resets in " + reset
  }

  onAccountsChanged: root.ensureSelection()

  Main {
    id: usage
    settings: root.settings
  }

  Timer {
    interval: 30000
    running: root.opened
    repeat: true
    onTriggered: usage.nowMs = Date.now()
  }

  KeyboardPanel {
    id: popup
    anchorItem: root.anchorItem
    owner: root.hostWidget || root
    bar: root.bar
    open: root.opened
    centerOnBar: true
    focusTarget: keyCatcher
    contentWidth: popup.fittedContentWidth(Style.space(480))
    contentHeight: popup.fittedContentHeight(column.implicitHeight, Style.space(840))

    FocusScope {
      id: panelFocus
      anchors.fill: parent
      focus: true

      function scrollToTop() {
        accountScroll.contentY = 0
      }

      function scrollToBottom() {
        accountScroll.contentY = Math.max(0, accountScroll.contentHeight - accountScroll.height)
      }

      function scrollByPage(direction) {
        var maximum = Math.max(0, accountScroll.contentHeight - accountScroll.height)
        var page = Math.max(Style.space(96), accountScroll.height * 0.85)
        accountScroll.contentY = Math.max(0, Math.min(
          maximum, accountScroll.contentY + direction * page))
      }

      // Keep Home/End and page navigation local to this panel. PanelKeyCatcher
      // intentionally leaves these keys unhandled so they reach this scope.
      Keys.priority: Keys.AfterItem
      Keys.onPressed: function(event) {
        if (event.key === Qt.Key_PageDown) {
          panelFocus.scrollByPage(1); event.accepted = true
        } else if (event.key === Qt.Key_PageUp) {
          panelFocus.scrollByPage(-1); event.accepted = true
        } else if (event.key === Qt.Key_Home) {
          panelFocus.scrollToTop(); event.accepted = true
        } else if (event.key === Qt.Key_End) {
          panelFocus.scrollToBottom(); event.accepted = true
        }
      }

      WheelHandler {
        id: panelWheel
        target: null
        orientation: Qt.Vertical
        enabled: accountScroll.contentHeight > accountScroll.height
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad

        onWheel: function(event) {
          var angle = Number(event.angleDelta.y)
          var pixels = Number(event.pixelDelta.y)
          var delta = 0

          if (angle !== 0) {
            var steps = Math.max(1, Math.abs(angle) / 120)
            delta = (angle > 0 ? -1 : 1) * Style.space(10) * 10 * steps
          } else if (pixels !== 0) {
            delta = -pixels * 1.5
          }

          if (delta === 0) return
          var maximum = Math.max(0, accountScroll.contentHeight - accountScroll.height)
          accountScroll.contentY = Math.max(0, Math.min(
            maximum, accountScroll.contentY + delta))
          event.accepted = true
        }
      }

      PanelKeyCatcher {
        id: keyCatcher
        anchors.fill: parent
        blocked: usage.desktopConfirmationOpen

        onMoveRequested: function(dx, dy) {
          if (dx !== 0) root.selectAccount(root.selectedIndex + dx)
          if (dy !== 0)
            accountScroll.contentY = Math.max(0, Math.min(
              accountScroll.contentHeight - accountScroll.height,
              accountScroll.contentY + dy * Style.space(56)))
        }
        onActivateRequested: usage.refresh()
        onCloseRequested: root.close()
        onTabRequested: function(direction) { root.switchPanel(direction) }
        onTextKey: function(text) {
          if (text === "r" || text === "R") usage.refresh()
          else if (text === "n" || text === "N") root.nextAccount()
        }

        ConfirmDialog {
          id: desktopConfirm
          anchors.fill: parent
          z: 20
          opened: usage.desktopConfirmationOpen
          message: usage.desktopConfirmationMessage
          confirmText: "Kill & switch"
          background: Color.background
          foreground: root.foreground
          scrim: Qt.rgba(Color.background.r, Color.background.g, Color.background.b, 0.7)
          selectedBackground: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.08)
          selectedText: Color.accent
          fontFamily: root.fontFamily
          cornerRadius: Style.cornerRadius
          focus: opened

          Keys.priority: Keys.BeforeItem
          Keys.onPressed: function(event) {
            if (desktopConfirm.handleKey(event)) event.accepted = true
          }

          onOpenedChanged: if (opened) forceActiveFocus()
          onCanceled: {
            usage.cancelDesktopSwitch()
            Qt.callLater(keyCatcher.forceActiveFocus)
          }
          onConfirmed: {
            usage.confirmDesktopSwitch()
            Qt.callLater(keyCatcher.forceActiveFocus)
          }
        }

        Flickable {
          id: accountScroll
          anchors.fill: parent
          contentWidth: width
          contentHeight: column.implicitHeight
          clip: true
          boundsBehavior: Flickable.StopAtBounds
          flickableDirection: Flickable.VerticalFlick
          interactive: contentHeight > height
          ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

          Column {
            id: column
            width: accountScroll.width
            spacing: Style.space(12)

          PanelHero {
            visible: !!root.selectedAccount
            width: parent.width
            title: root.selectedAccount ? String(root.selectedAccount.name || "Account") : ""
            meta: root.selectedAccount
              ? usage.accountMeta(root.selectedAccount)
              : ""
            foreground: root.foreground
            fontFamily: root.fontFamily

            iconComponent: Component {
              Text {
                text: "\uf915"
                color: root.foreground
                font.family: "bootstrap-icons"
                font.pixelSize: Style.font.display
              }
            }
            trailingControl: Component {
              PanelActionButton {
                size: Style.space(26)
                iconText: "↓"
                tooltipText: "Privacy"
                foreground: root.foreground
                hoverColor: Color.accent
                onClicked: panelFocus.scrollToBottom()
              }
            }
          }

          Text {
            visible: !root.selectedAccount && usage.errorText === ""
            width: parent.width
            text: "No Codex or PI accounts found."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
          }

          BorderSurface {
            visible: usage.errorText !== "" || usage.actionStatusText !== ""
            width: parent.width
            implicitHeight: statusLabel.implicitHeight + Style.space(20)
            color: Qt.rgba(Color.urgent.r, Color.urgent.g, Color.urgent.b, 0.10)
            borderSpec: Border.flat(Qt.rgba(Color.urgent.r, Color.urgent.g, Color.urgent.b, 0.35), 1)
            radius: Style.cornerRadius

            Text {
              id: statusLabel
              anchors.left: parent.left
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              anchors.leftMargin: Style.space(12)
              anchors.rightMargin: Style.space(12)
              text: usage.actionStatusText !== "" ? usage.actionStatusText : usage.errorText
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              wrapMode: Text.WordWrap
            }
          }

          Row {
            id: accountSelector
            visible: root.accounts.length > 1
            width: parent.width
            spacing: Style.spacing.md

            readonly property real cellWidth: root.accounts.length > 0
              ? (width - spacing * (root.accounts.length - 1)) / root.accounts.length : 0

            Repeater {
              model: root.accounts

              Button {
                required property var modelData
                required property int index
                width: accountSelector.cellWidth
                text: String(modelData.name || "Account")
                selected: index === root.selectedIndex
                hasCursor: root.cursorActive && index === root.selectedIndex
                bordered: true
                foreground: root.foreground
                fontFamily: root.fontFamily
                fontSize: Style.font.bodySmall
                verticalPadding: Style.spacing.controlPaddingY
                onClicked: {
                  root.cursorActive = true
                  root.selectedKey = modelData.key
                }
                onHovered: function(isHovered) { if (isHovered) root.cursorActive = true }
              }
            }
          }

          PanelSeparator {
            visible: !!root.selectedAccount
            foreground: root.foreground
          }

          Column {
            visible: !!root.selectedAccount
            width: parent.width
            spacing: Style.space(10)

            PanelSectionHeader {
              width: parent.width
              text: "ACCOUNT SOURCES"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Repeater {
              model: root.selectedAccount ? (root.selectedAccount.sources || []) : []

              Item {
                required property var modelData
                width: parent.width
                implicitHeight: sourceRow.implicitHeight + Style.space(4)

                Row {
                  id: sourceRow
                  width: parent.width
                  spacing: Style.space(8)

                  Text {
                    text: String(modelData.provider || "").toUpperCase()
                      + " · " + String(modelData.profile || "?")
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.bodySmall
                    elide: Text.ElideRight
                    width: parent.width * 0.50
                  }

                  Text {
                    text: modelData.live === true
                      ? (modelData.status === "stale" ? "CURRENT · CACHED" : "CURRENT")
                      : String(modelData.status || "").toUpperCase()
                    color: modelData.status === "unavailable" ? Color.urgent : root.dim
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    width: parent.width * 0.20
                    horizontalAlignment: Text.AlignRight
                  }

                  Button {
                    text: modelData.live === true ? "Current" : "Use"
                    enabled: modelData.switchable === true && !usage.switching && !usage.updating
                      && !usage.desktopSwitchPending
                    bordered: true
                    foreground: root.foreground
                    fontFamily: root.fontFamily
                    fontSize: Style.font.caption
                    verticalPadding: Style.space(3)
                    onClicked: usage.switchSource(modelData)
                  }
                }

                Text {
                  visible: modelData.status !== "ok"
                  anchors.top: sourceRow.bottom
                  anchors.topMargin: Style.space(2)
                  width: parent.width
                  text: String(modelData.error || "Usage unavailable")
                  color: modelData.status === "unavailable" ? Color.urgent : root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  elide: Text.ElideRight
                }
              }
            }
          }

          PanelSeparator {
            visible: !!root.selectedQuota
            foreground: root.foreground
          }

          Column {
            visible: !!root.selectedQuota
            width: parent.width
            spacing: Style.space(10)

            PanelSectionHeader {
              width: parent.width
              text: "QUOTAS"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Text {
              width: parent.width
              text: usage.fetchLabel(root.selectedAccount)
              color: root.selectedAccount && root.selectedAccount.status === "stale"
                ? Color.urgent : root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }

            Repeater {
              model: root.selectedQuota ? (root.selectedQuota.windows || []) : []

              Column {
                required property var modelData
                width: parent.width
                spacing: Style.space(6)

                Item {
                  width: parent.width
                  implicitHeight: Math.max(windowLabel.implicitHeight, windowValue.implicitHeight)

                  Text {
                    id: windowLabel
                    text: String(modelData.kind || "window").toUpperCase()
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                  }

                  Text {
                    id: windowValue
                    text: root.percentLabel(modelData)
                    color: Number(modelData.usedPercent || 0) >= 90 ? Color.urgent : root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                  }
                }

                Rectangle {
                  width: parent.width
                  height: Math.max(Style.space(4), Math.round(Style.spacing.controlHeight * 0.14))
                  radius: height / 2
                  color: root.track

                  Rectangle {
                    width: {
                      var value = usage.percentValue(modelData)
                      return parent.width * (value === null
                        ? 0 : Math.max(0, Math.min(1, Number(value) / 100)))
                    }
                    height: parent.height
                    radius: parent.radius
                    color: Number(modelData.usedPercent || 0) >= 90 ? Color.urgent : root.foreground
                  }
                }

                Text {
                  visible: root.resetLabel(modelData) !== ""
                  text: root.resetLabel(modelData)
                  color: root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                }
              }
            }

            Column {
              id: monthlyColumn
              visible: !!root.selectedQuota && !!root.selectedQuota.monthly
              width: parent.width
              spacing: Style.space(6)

              Text {
                text: "MONTHLY CREDITS"
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: true
              }

              Rectangle {
                width: parent.width
                height: Math.max(Style.space(4), Math.round(Style.spacing.controlHeight * 0.14))
                radius: height / 2
                color: root.track

                Rectangle {
                  width: {
                    var monthly = root.selectedQuota ? root.selectedQuota.monthly : null
                    var value = monthly ? usage.percentValue(monthly) : null
                    return parent.width * (value === null
                      ? 0 : Math.max(0, Math.min(1, Number(value) / 100)))
                  }
                  height: parent.height
                  radius: parent.radius
                  color: {
                    var monthly = root.selectedQuota ? root.selectedQuota.monthly : null
                    return Number(monthly && monthly.usedPercent || 0) >= 90
                      ? Color.urgent : root.foreground
                  }
                }
              }

              Text {
                text: {
                  var monthly = root.selectedQuota ? root.selectedQuota.monthly : null
                  return monthly ? String(monthly.used === null || monthly.used === undefined ? "?" : monthly.used)
                    + " / " + String(monthly.limit === null || monthly.limit === undefined ? "?" : monthly.limit)
                    + " used (" + usage.formatPercent(monthly.usedPercent) + ")" : ""
                }
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
              }

              Text {
                text: {
                  var monthly = root.selectedQuota ? root.selectedQuota.monthly : null
                  return monthly ? String(monthly.remaining === null || monthly.remaining === undefined ? "?" : monthly.remaining)
                    + " credits left (" + usage.formatPercent(monthly.remainingPercent) + ")"
                    + (monthly.reached === true ? " · limit reached" : "") : ""
                }
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }

              Text {
                visible: !!root.selectedQuota && !!root.selectedQuota.monthly
                  && !!root.selectedQuota.monthly.resetAt
                text: root.selectedQuota && root.selectedQuota.monthly
                  ? "Resets in " + usage.formatDuration(root.selectedQuota.monthly.resetAt) : ""
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }

            Column {
              id: tokenUsageColumn
              visible: {
                var tokenUsage = root.selectedQuota ? root.selectedQuota.tokenUsage : null
                return !!tokenUsage && Number(tokenUsage.days || 0) > 0
              }
              width: parent.width
              spacing: Style.space(6)

              PanelSectionHeader {
                width: parent.width
                text: {
                  var tokenUsage = root.selectedQuota ? root.selectedQuota.tokenUsage : null
                  return "TOKENS USED · " + String(tokenUsage && tokenUsage.days || 0) + "D"
                }
                foreground: root.foreground
                fontFamily: root.fontFamily
              }

              Text {
                width: parent.width
                text: {
                  var tokenUsage = root.selectedQuota ? root.selectedQuota.tokenUsage : null
                  return usage.formatTokenCount(tokenUsage ? tokenUsage.totalTokens : null)
                    + " tokens"
                }
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
              }

              Text {
                visible: {
                  var tokenUsage = root.selectedQuota ? root.selectedQuota.tokenUsage : null
                  return !!tokenUsage && tokenUsage.peakDailyTokens !== null
                    && tokenUsage.peakDailyTokens !== undefined
                }
                text: {
                  var tokenUsage = root.selectedQuota ? root.selectedQuota.tokenUsage : null
                  return "Peak day: " + usage.formatTokenCount(
                    tokenUsage ? tokenUsage.peakDailyTokens : null) + " tokens"
                }
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }

            Column {
              id: modelUsageColumn
              visible: {
                var modelUsage = root.selectedQuota ? root.selectedQuota.modelUsage : null
                return !!modelUsage && Array.isArray(modelUsage.models)
                  && modelUsage.models.length > 0
              }
              width: parent.width
              spacing: Style.space(6)

              PanelSectionHeader {
                width: parent.width
                text: {
                  var modelUsage = root.selectedQuota ? root.selectedQuota.modelUsage : null
                  var rawUnits = modelUsage && modelUsage.units
                    ? String(modelUsage.units).toLowerCase() : ""
                  var units = rawUnits === "percent"
                    ? " · PERCENTAGE POINTS"
                    : (rawUnits === "" ? "" : " · " + rawUnits.toUpperCase())
                  return "DAILY MODEL USAGE · " + String(modelUsage && modelUsage.days || 0) + "D"
                    + units
                }
                foreground: root.foreground
                fontFamily: root.fontFamily
              }

              ModelUsageChart {
                id: modelUsageChart
                width: parent.width
                scopeKey: root.selectedAccount ? root.selectedAccount.key : ""
                daily: root.selectedQuota && root.selectedQuota.modelUsage
                  ? root.selectedQuota.modelUsage.daily : []
                series: root.selectedQuota && root.selectedQuota.modelUsage
                  ? root.selectedQuota.modelUsage.models : []
                liveUsage: {
                  var quota = root.selectedQuota
                  if (!quota) return false
                  var windows = Array.isArray(quota.windows) ? quota.windows : []
                  for (var i = 0; i < windows.length; i++) {
                    if (Number(windows[i].usedPercent || 0) > 0) return true
                  }
                  return !!quota.monthly && Number(quota.monthly.usedPercent || 0) > 0
                }
                foreground: root.foreground
                dim: root.dim
                track: root.track
              }

              PanelSeparator {
                foreground: root.foreground
              }

              PanelSectionHeader {
                width: parent.width
                text: {
                  var modelUsage = root.selectedQuota ? root.selectedQuota.modelUsage : null
                  return "MODEL TOTALS"
                    + (modelUsage && String(modelUsage.units || "").toLowerCase() === "percent"
                      ? " · PERCENTAGE POINTS" : "")
                }
                foreground: root.foreground
                fontFamily: root.fontFamily
              }

              Repeater {
                model: {
                  var modelUsage = root.selectedQuota ? root.selectedQuota.modelUsage : null
                  return modelUsage && Array.isArray(modelUsage.models) ? modelUsage.models : []
                }

                Item {
                  required property var modelData
                  width: parent.width
                  implicitHeight: modelRow.implicitHeight

                  Row {
                    id: modelRow
                    width: parent.width
                    spacing: Style.space(8)

                    Text {
                      width: (parent.width - parent.spacing) * 0.74
                      text: String(modelData.model || "model")
                        + (modelData.speed ? " · " + String(modelData.speed) : "")
                      color: root.foreground
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.bodySmall
                      elide: Text.ElideRight
                    }

                    Text {
                      width: (parent.width - parent.spacing) * 0.26
                      text: usage.formatModelUsage(
                        modelData.credits,
                        root.selectedQuota && root.selectedQuota.modelUsage
                          ? root.selectedQuota.modelUsage.units : ""
                      )
                      color: root.foreground
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.bodySmall
                      horizontalAlignment: Text.AlignRight
                    }
                  }
                }
              }
            }
          }

          Column {
            visible: !!root.selectedQuota && !!root.selectedQuota.resetCredits
            width: parent.width
            spacing: Style.space(6)

            PanelSectionHeader {
              width: parent.width
              text: "RESET CREDITS"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Text {
              width: parent.width
              text: {
                var credits = root.selectedQuota ? root.selectedQuota.resetCredits : null
                if (!credits) return ""
                return String(credits.availableCount || 0) + " available · "
                  + String(credits.applicableAvailableCount || 0) + " currently applicable"
              }
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
            }

            Repeater {
              model: root.selectedQuota && root.selectedQuota.resetCredits
                ? (root.selectedQuota.resetCredits.credits || []) : []

              Text {
                required property var modelData
                width: parent.width
                text: {
                  var title = String(modelData.title || "Reset")
                  if (!modelData.expiresAt) return "• " + title
                  var remaining = usage.formatDuration(modelData.expiresAt)
                  return "• " + title + " · "
                    + (remaining === "" ? "" : "in " + remaining + " · ")
                    + usage.formatExpiry(modelData.expiresAt)
                }
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                wrapMode: Text.Wrap
              }
            }
          }

          Text {
            visible: !!(usage.snapshot.lastQuotaHit && usage.snapshot.lastQuotaHit.profile)
            width: parent.width
            text: usage.snapshot.lastQuotaHit
              ? "Last quota hit: " + String(usage.snapshot.lastQuotaHit.profile || "?")
                + " · " + String(usage.snapshot.lastQuotaHit.window || "?")
                + " · " + usage.formatPercent(usage.snapshot.lastQuotaHit.usedPercent)
              : ""
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
          }

          PanelSeparator {
            foreground: root.foreground
          }

          Column {
            width: parent.width
            spacing: Style.space(6)

            PanelSectionHeader {
              width: parent.width
              text: "PRIVACY"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Row {
              width: parent.width
              spacing: Style.space(4)

              Column {
                width: parent.width
                spacing: Style.space(4)

                Text {
                  width: parent.width
                  text: usage.emailPrivacyText
                  color: usage.emailsHidden ? root.foreground : root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                  elide: Text.ElideRight
                }

                Text {
                  width: parent.width
                  text: usage.streamingStatusText
                  color: usage.streamingDetected ? Color.urgent : root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  elide: Text.ElideRight
                }
              }
            }

            Row {
              width: parent.width
              spacing: Style.space(8)

              Button {
                id: hideEmailsButton
                text: usage.hideEmailsSetting ? "Allow emails" : "Hide emails"
                selected: usage.hideEmailsSetting
                enabled: !usage.privacySettingRunning
                  && !usage.updating && !usage.switching
                bordered: true
                focusable: true
                foreground: root.foreground
                fontFamily: root.fontFamily
                fontSize: Style.font.caption
                verticalPadding: Style.space(3)
                tooltipText: usage.hideEmailsSetting
                  ? "Allow account emails when auto-hide is not active"
                  : "Always hide account emails"
                onClicked: usage.toggleHideEmails()
              }

              Button {
                id: streamingCheckButton
                text: usage.streamingCheckRunning ? "Checking..." : "Check streaming"
                enabled: !usage.streamingCheckRunning
                  && !usage.privacySettingRunning
                  && !usage.updating && !usage.switching
                bordered: true
                focusable: true
                foreground: root.foreground
                fontFamily: root.fontFamily
                fontSize: Style.font.caption
                verticalPadding: Style.space(3)
                onClicked: usage.checkStreaming()
              }
            }

            Text {
              width: parent.width
              text: usage.autoHideEmailsWhenStreaming
                ? "Auto-hide is enabled; the button controls permanent hiding."
                : "The button stores the permanent hide setting; auto-hide is in plugin settings."
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              wrapMode: Text.WordWrap
            }
          }
          }
        }
      }
    }
  }
}
