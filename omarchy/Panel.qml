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
  readonly property string barTooltip: usage.barTooltipValue
  readonly property bool alarming: usage.alarming(usage.activeAccount)

  function open() {
    usage.refresh()
    root.controller.show()
    Qt.callLater(function() {
      if (root.opened) keyCatcher.forceActiveFocus()
    })
  }

  function close() {
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
    contentWidth: popup.fittedContentWidth(Style.space(390))
    contentHeight: popup.fittedContentHeight(column.implicitHeight, Style.space(650))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent

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
              ? String(root.selectedAccount.email || "")
                + (root.selectedAccount.current ? " · current" : "")
              : ""
            foreground: root.foreground
            fontFamily: root.fontFamily

            iconComponent: Component {
              Text {
                text: "󱚣"
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.display
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
                    text: modelData.live === true ? "CURRENT" : String(modelData.status || "")
                    color: modelData.status === "ok" ? root.dim : Color.urgent
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    width: parent.width * 0.20
                    horizontalAlignment: Text.AlignRight
                  }

                  Button {
                    text: modelData.live === true ? "Current" : "Use"
                    enabled: modelData.switchable === true && !usage.switching && !usage.updating
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
                  color: Color.urgent
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

            BorderSurface {
              visible: !!root.selectedQuota && !!root.selectedQuota.monthly
              width: parent.width
              implicitHeight: monthlyColumn.implicitHeight + Style.space(18)
              color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.05)
              borderSpec: Border.flat(Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15), 1)
              radius: Style.cornerRadius

              Column {
                id: monthlyColumn
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: Style.space(9)
                spacing: Style.space(4)

                Text {
                  text: "MONTHLY CREDITS"
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.bold: true
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
                text: "• " + String(modelData.title || "Reset")
                  + (modelData.expiresAt ? " · expires " + String(modelData.expiresAt) : "")
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                elide: Text.ElideRight
              }
            }
          }

          Text {
            visible: usage.snapshot.lastQuotaHit && usage.snapshot.lastQuotaHit.profile
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
        }
      }
    }
  }
}
