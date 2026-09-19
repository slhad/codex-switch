import QtQuick
import qs.Commons
import qs.Ui

Item {
  id: root

  property var daily: []
  property var series: []
  property string scopeKey: ""
  property int rangeDays: 7
  property var hiddenSeriesKeys: []
  property bool liveUsage: false
  property color foreground: Color.foreground
  property color dim: Qt.darker(foreground, 1.55)
  property color track: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.16)
  property real plotHeight: Style.space(110)

  readonly property var rangeDaily: root.dailyForRange()
  readonly property var legendSeries: root.seriesForLegend()
  readonly property var chartSeries: root.seriesForChart()
  readonly property bool hasRangeData: root.hasPositiveUsage(root.rangeDaily, root.legendSeries)
  readonly property bool hasData: root.rangeDaily.length > 0
    && root.chartSeries.length > 0 && root.maxValue > 0
  readonly property real maxValue: {
    var maximum = 0
    for (var i = 0; i < root.rangeDaily.length; i++) {
      var total = root.dayTotal(root.rangeDaily[i])
      if (total > maximum) maximum = total
    }
    return maximum
  }

  implicitHeight: content.implicitHeight

  onDailyChanged: chart.requestPaint()
  onSeriesChanged: chart.requestPaint()
  onRangeDaysChanged: chart.requestPaint()
  onHiddenSeriesKeysChanged: chart.requestPaint()
  onForegroundChanged: chart.requestPaint()
  onTrackChanged: chart.requestPaint()
  onScopeKeyChanged: root.hiddenSeriesKeys = []

  function numberValue(value) {
    var number = Number(value)
    return isFinite(number) && number > 0 ? number : 0
  }

  function seriesKey(value) {
    if (!value) return ""
    return String(value.model || "") + "\u001f" + String(value.speed || "")
  }

  function dateKey(value) {
    var date = value instanceof Date ? value : new Date(value)
    if (!isFinite(date.getTime())) return ""
    function pad(number) { return number < 10 ? "0" + number : String(number) }
    return String(date.getUTCFullYear()) + "-" + pad(date.getUTCMonth() + 1)
      + "-" + pad(date.getUTCDate())
  }

  function dayDateKey(day) {
    return day && day.date ? String(day.date).slice(0, 10) : ""
  }

  function dayHasUsage(day) {
    var values = day && Array.isArray(day.models) ? day.models : []
    for (var i = 0; i < values.length; i++) {
      if (root.numberValue(values[i].credits) > 0) return true
    }
    return false
  }

  function dailyForRange() {
    var values = Array.isArray(root.daily) ? root.daily.slice() : []
    var today = root.dateKey(new Date())
    values = values.filter(function(day) {
      var date = root.dayDateKey(day)
      return date !== "" && (today === "" || date <= today)
    })
    var limit = Math.floor(Number(root.rangeDays))
    var lastReported = -1
    for (var reportedIndex = 0; reportedIndex < values.length; reportedIndex++) {
      if (root.dayHasUsage(values[reportedIndex])) lastReported = reportedIndex
    }

    var keepEmptyToday = false
    if (root.rangeDays === 1 && root.liveUsage) {
      var hasToday = false
      var todayHasUsage = false
      for (var todayIndex = 0; todayIndex < values.length; todayIndex++) {
        if (root.dayDateKey(values[todayIndex]) !== today) continue
        hasToday = true
        todayHasUsage = todayHasUsage || root.dayHasUsage(values[todayIndex])
      }
      keepEmptyToday = hasToday && !todayHasUsage
    }

    if (lastReported >= 0 && lastReported < values.length - 1) {
      values = values.slice(0, lastReported + 1)
      if (keepEmptyToday) {
        for (var pendingIndex = 0; pendingIndex < root.daily.length; pendingIndex++) {
          if (root.dayDateKey(root.daily[pendingIndex]) === today)
            values.push(root.daily[pendingIndex])
        }
      }
    }
    if (!isFinite(limit) || limit <= 0 || values.length <= limit) return values
    return values.slice(values.length - limit)
  }

  function noDataText() {
    var lastDay = root.rangeDaily.length > 0
      ? root.rangeDaily[root.rangeDaily.length - 1] : null
    var currentDayPending = root.rangeDays === 1 && root.liveUsage
      && root.dayDateKey(lastDay) === root.dateKey(new Date())
      && !root.hasRangeData
    if (currentDayPending) return "Live usage detected · daily breakdown pending"
    return root.hasRangeData ? "All models hidden" : "No model usage in selected range"
  }

  function valueFor(day, target) {
    var values = day && Array.isArray(day.models) ? day.models : []
    var targetKey = seriesKey(target)
    for (var i = 0; i < values.length; i++) {
      if (seriesKey(values[i]) === targetKey) return numberValue(values[i].credits)
    }
    return 0
  }

  function dayTotal(day) {
    var total = 0
    for (var i = 0; i < root.chartSeries.length; i++)
      total += valueFor(day, root.chartSeries[i])
    return total
  }

  function seriesIndex(target) {
    if (!Array.isArray(root.series)) return -1
    var targetKey = seriesKey(target)
    for (var i = 0; i < root.series.length; i++) {
      if (seriesKey(root.series[i]) === targetKey) return i
    }
    return -1
  }

  function seriesHasValue(days, target) {
    if (!Array.isArray(days)) return false
    for (var i = 0; i < days.length; i++) {
      if (valueFor(days[i], target) > 0) return true
    }
    return false
  }

  function hasPositiveUsage(days, candidates) {
    if (!Array.isArray(candidates)) return false
    for (var i = 0; i < candidates.length; i++) {
      if (seriesHasValue(days, candidates[i])) return true
    }
    return false
  }

  function seriesForLegend() {
    var all = Array.isArray(root.series) ? root.series : []
    var values = []
    for (var i = 0; i < all.length; i++) {
      if (root.seriesHasValue(root.rangeDaily, all[i])) values.push(all[i])
    }
    return values.length > 0 ? values : all
  }

  function seriesShown(target) {
    var key = root.seriesKey(target)
    if (key === "") return true
    var hidden = Array.isArray(root.hiddenSeriesKeys) ? root.hiddenSeriesKeys : []
    return hidden.indexOf(key) < 0
  }

  function seriesForChart() {
    var values = []
    var candidates = root.seriesForLegend()
    for (var i = 0; i < candidates.length; i++) {
      if (root.seriesShown(candidates[i])
          && root.seriesHasValue(root.rangeDaily, candidates[i]))
        values.push(candidates[i])
    }
    return values
  }

  function toggleSeries(target) {
    var key = root.seriesKey(target)
    if (key === "") return

    var next = []
    var wasHidden = false
    var hidden = Array.isArray(root.hiddenSeriesKeys) ? root.hiddenSeriesKeys : []
    for (var i = 0; i < hidden.length; i++) {
      if (hidden[i] === key) wasHidden = true
      else next.push(hidden[i])
    }
    if (!wasHidden) next.push(key)
    root.hiddenSeriesKeys = next
  }

  function seriesColor(index) {
    var palette = [
      root.foreground,
      Color.accent,
      Qt.lighter(Color.accent, 1.25),
      Qt.darker(root.foreground, 1.45),
      Color.urgent,
      Qt.lighter(root.foreground, 1.25)
    ]
    var safeIndex = Math.max(0, Number(index) || 0)
    return palette[safeIndex % palette.length]
  }

  function cssColor(color, alpha) {
    return "rgba(" + Math.round(color.r * 255) + ","
      + Math.round(color.g * 255) + ","
      + Math.round(color.b * 255) + "," + alpha + ")"
  }

  function paintChart(context, canvasWidth, canvasHeight) {
    context.clearRect(0, 0, canvasWidth, canvasHeight)
    context.lineWidth = 1
    context.strokeStyle = root.cssColor(root.track, 1)
    context.beginPath()
    for (var gridIndex = 0; gridIndex < 3; gridIndex++) {
      var gridY = gridIndex === 0 ? 0
        : (gridIndex === 1 ? Math.round(canvasHeight / 2) : canvasHeight - 1)
      context.moveTo(0, gridY)
      context.lineTo(canvasWidth, gridY)
    }
    context.stroke()

    if (!root.hasData) return

    var days = root.rangeDaily
    var plottedSeries = root.chartSeries
    var dayCount = days.length
    var lastDay = dayCount - 1
    for (var seriesIndex = 0; seriesIndex < plottedSeries.length; seriesIndex++) {
      var upper = []
      var lower = []
      var hasValue = false

      for (var dayIndex = 0; dayIndex < dayCount; dayIndex++) {
        var day = days[dayIndex]
        var bottom = 0
        for (var previous = 0; previous < seriesIndex; previous++)
          bottom += valueFor(day, plottedSeries[previous])
        var value = valueFor(day, plottedSeries[seriesIndex])
        var top = bottom + value
        if (value > 0) hasValue = true

        var x = lastDay > 0 ? canvasWidth * dayIndex / lastDay : canvasWidth / 2
        lower.push([x, canvasHeight - bottom / root.maxValue * canvasHeight])
        upper.push([x, canvasHeight - top / root.maxValue * canvasHeight])
      }

      if (!hasValue) continue

      context.beginPath()
      context.moveTo(upper[0][0], upper[0][1])
      for (var upperIndex = 1; upperIndex < upper.length; upperIndex++)
        context.lineTo(upper[upperIndex][0], upper[upperIndex][1])
      for (var lowerIndex = lower.length - 1; lowerIndex >= 0; lowerIndex--)
        context.lineTo(lower[lowerIndex][0], lower[lowerIndex][1])
      context.closePath()
      var colorIndex = root.seriesIndex(plottedSeries[seriesIndex])
      context.fillStyle = root.cssColor(root.seriesColor(colorIndex), 0.68)
      context.fill()

      context.beginPath()
      context.moveTo(upper[0][0], upper[0][1])
      for (var lineIndex = 1; lineIndex < upper.length; lineIndex++)
        context.lineTo(upper[lineIndex][0], upper[lineIndex][1])
      context.strokeStyle = root.cssColor(root.seriesColor(colorIndex), 0.92)
      context.stroke()
    }
  }

  function formatDate(value) {
    var date = new Date(String(value || "") + "T00:00:00")
    if (!isFinite(date.getTime())) return String(value || "?")
    return String(date.getMonth() + 1) + "/" + String(date.getDate())
  }

  Column {
    id: content
    width: parent.width
    spacing: Style.space(4)

    Item {
      id: rangeToolbar
      width: parent.width
      implicitHeight: rangeSelector.implicitHeight

      Text {
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        text: "RANGE"
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      ButtonGroup {
        id: rangeSelector
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        options: [
          { value: "1", label: "1D", tooltip: "Show the latest reported day" },
          { value: "7", label: "7D", tooltip: "Show the last 7 days" },
          { value: "30", label: "30D", tooltip: "Show the last 30 days" }
        ]
        value: String(root.rangeDays)
        foreground: root.dim
        background: "transparent"
        fontFamily: Style.font.family
        fontSize: Style.font.caption
        focusable: false
        onChanged: function(value) { root.rangeDays = Number(value) }
      }
    }

    Item {
      id: plot
      width: parent.width
      height: root.plotHeight

      Canvas {
        id: chart
        anchors.fill: parent
        renderTarget: Canvas.FramebufferObject

        onPaint: {
          root.paintChart(getContext("2d"), width, height)
        }
      }

      Text {
        anchors.centerIn: parent
        visible: !root.hasData
        text: root.noDataText()
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }
    }

    Item {
      id: dateLabels
      width: parent.width
      implicitHeight: Math.max(startDate.implicitHeight,
        Math.max(middleDate.implicitHeight, endDate.implicitHeight))
      visible: root.rangeDaily.length > 0

      Text {
        id: startDate
        anchors.left: parent.left
        text: root.formatDate(root.rangeDaily[0] ? root.rangeDaily[0].date : "")
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      Text {
        id: middleDate
        anchors.horizontalCenter: parent.horizontalCenter
        text: root.formatDate(root.rangeDaily.length > 0
          ? root.rangeDaily[Math.floor((root.rangeDaily.length - 1) / 2)].date : "")
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      Text {
        id: endDate
        anchors.right: parent.right
        text: root.formatDate(root.rangeDaily.length > 0
          ? root.rangeDaily[root.rangeDaily.length - 1].date : "")
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }
    }

    Flow {
      id: legend
      width: parent.width
      spacing: Style.space(8)

      Repeater {
        model: root.legendSeries

        Button {
          required property var modelData
          required property int index
          property int sourceIndex: root.seriesIndex(modelData)
          text: String(modelData.model || "model")
            + (modelData.speed ? " · " + String(modelData.speed) : "")
          iconText: "\u2022"
          active: root.seriesShown(modelData)
          foreground: root.seriesShown(modelData)
            ? root.seriesColor(sourceIndex) : root.dim
          background: "transparent"
          bordered: false
          fontFamily: Style.font.family
          fontSize: Style.font.caption
          horizontalPadding: Style.space(2)
          verticalPadding: Style.space(2)
          tooltipText: root.seriesShown(modelData) ? "Hide this model" : "Show this model"
          onClicked: root.toggleSeries(modelData)
        }
      }
    }
  }
}
