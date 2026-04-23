import { createChart } from 'lightweight-charts'

window.theme = localStorage.getItem("theme") ?? "dark"
window.locale = navigator.language ?? navigator.userLanguage
window.magnet = localStorage.getItem("magnet") ?? true
window.showVolume = localStorage.getItem("showVolume") ?? true

function initChart() {
  class ViewManager {
    attached(primitive) {
      this.primitive = primitive
      this.layers = {}
      this.list = []
    }

    paneViews() {
      return this.list
    }

    updateLayer(layerName, ...list) {
      this.layers[layerName] = list.map(v => {
        v.renderer = () => v

        return v
      })

      this.list = Object.values(this.layers).flat()

      this.primitive.requestUpdate()
    }

    clearLayer(layerName) {
      if (!this.layers[layerName]) {
        return
      }

      delete this.layers[layerName]
      this.list = Object.values(this.layers).flat()

      this.primitive.requestUpdate()
    }
  }

  window.chart = createChart(document.getElementById('chart-container'))
  window.series = window.chart.addCandlestickSeries()
  window.series.attachPrimitive(window.vm = new ViewManager())
  window.volumeSeries = window.chart.addHistogramSeries({
    lastValueVisible: false,
    priceLineVisible: false,
    priceFormat: {
      type: 'volume',
    },
    priceScaleId: 'volume',
  })

  window.volumeSeries.priceScale().applyOptions({
    scaleMargins: {
      top: 0.8,
      bottom: 0,
    },
  })

  return [window.chart, window.series, window.vm, window.volumeSeries]
}

function applyOptions() {
  const body = document.querySelector("body")

  body.classList.forEach(className => {
    if (className.startsWith("theme-")) {
      body.classList.remove(className)
    }
  })

  body.classList.add(`theme-${window.theme}`)

  const style = getComputedStyle(body)
  const buyColor = style.getPropertyValue('--buy-color').trim()
  const sellColor = style.getPropertyValue('--sell-color').trim()

  const tickMarkFormatter = (time, tickMarkType, locale) => {
    let date

    if (typeof time === 'string') {
      date = new Date(time)
    } else if (typeof time === 'number') {
      date = new Date(time)
    } else {
      date = new Date(time.year, time.month - 1, time.day)
    }

    const TickMarkType = {
      Year: 0,
      Month: 1,
      DayOfMonth: 2,
      Time: 3,
      TimeWithSeconds: 4
    }

    switch (tickMarkType) {
      case TickMarkType.Year:
        return date.toLocaleString(locale, { year: 'numeric' }).slice(0, 4)

      case TickMarkType.Month:
        return date.toLocaleString(locale, { month: 'short' }).slice(0, 3)

      case TickMarkType.DayOfMonth:
        return date.toLocaleString(locale, { day: '2-digit' }).padStart(2, '0')

      case TickMarkType.Time:
        return date.toLocaleString(locale, { hour: '2-digit', minute: '2-digit' }).slice(0, 5)

      case TickMarkType.TimeWithSeconds:
        return date.toLocaleString(locale, { hour: '2-digit', minute: '2-digit', second: '2-digit' }).slice(0, 8)

      default:
        return null
    }
  }

  const timeFormatter = (time) => {
    let date

    if (typeof time === 'string') {
      date = new Date(time)
    } else if (typeof time === 'number') {
      date = new Date(time)
    } else {
      date = new Date(time.year, time.month - 1, time.day)
    }

    const year = date.getFullYear()
    const month = String(date.getMonth() + 1).padStart(2, '0')
    const day = String(date.getDate()).padStart(2, '0')
    const hour = String(date.getHours()).padStart(2, '0')
    const minute = String(date.getMinutes()).padStart(2, '0')
    const second = String(date.getSeconds()).padStart(2, '0')
    const ampm = hour >= 12 ? 'PM' : 'AM'
    const hour12 = hour % 12 || 12

    const dateFormat = {
      "zh-CN": "yyyy-MM-dd HH:mm",
      "en-US": "MMM dd, yyyy hh:mm A",
      "en-GB": "dd MMM yyyy HH:mm",
      "en-CA": "yyyy-MM-dd HH:mm",
      "en-AU": "dd/MM/yyyy HH:mm",
      "fr-FR": "dd/MM/yyyy HH:mm",
      "de-DE": "dd.MM.yyyy HH:mm",
      "ja-JP": "yyyy-MM-dd HH:mm",
      "ko-KR": "yyyy.MM.dd HH:mm",
      "ru-RU": "dd.MM.yyyy HH:mm",
      "es-ES": "dd/MM/yyyy HH:mm",
      "it-IT": "dd/MM/yyyy HH:mm",
    }[locale] || "yyyy-MM-dd HH:mm"

    return dateFormat
      .replace('yyyy', year)
      .replace('MM', month)
      .replace('dd', day)
      .replace('HH', hour)
      .replace('hh', hour12)
      .replace('mm', minute)
      .replace('ss', second)
      .replace('A', ampm)
  }

  const priceFormatter = v => {
    const tick_size = window.dataSource.metadata.tick_size;
    const snapped = Math.round(v / tick_size) * tick_size;
    const precision = (tick_size.toString().split('.')[1] || '').length;

    return snapped.toFixed(precision);
  }

  chart.applyOptions({
    layout: {
      background: { color: style.getPropertyValue('--background-color') },
      textColor: style.getPropertyValue('--label-color'),
    },
    grid: {
      vertLines: {
        color: style.getPropertyValue('--grid-color'),
      },
      horzLines: {
        color: style.getPropertyValue('--grid-color'),
      },
    },
    crosshair: {
      mode: 0,
      horzLine: {
        labelBackgroundColor: style.getPropertyValue("--border-color") /* "#363a45" */
      },
      vertLine: {
        labelBackgroundColor: style.getPropertyValue("--border-color")
      },
    },
    timeScale: {
      borderColor: style.getPropertyValue("--border-color"),
      tickMarkFormatter,
    },
    rightPriceScale: {
      borderColor: style.getPropertyValue("--border-color")
    },
    localization: {
      timeFormatter,
      priceFormatter,
    },
    autoSize: true,
  })

  window.series.applyOptions({
    upColor: buyColor,
    downColor: sellColor,
    borderUpColor: buyColor,
    borderDownColor: sellColor,
    wickUpColor: buyColor,
    wickDownColor: sellColor,
    lastValueVisible: false,
    priceLineVisible: false,
    priceFormat: {
      type: "custom",
      formatter: priceFormatter,
      minMove: window.dataSource.metadata.tick_size || 0.01,
    }
  })

  window.chart.subscribeCrosshairMove(({ seriesData, point, time, logical, hoveredObjectId }) => {
    if (hoveredObjectId) {
      document.body.style.cursor = 'pointer'
    } else {
      document.body.style.cursor = 'default'
    }

    const k = seriesData.get(window.series)

    if (!k) {
      return
    }

    const y = point.y
    const { open, high, low, close } = k
    const volume = window.dataSource.data[logical].volume

    const list = [
      { price: open, distance: Math.abs(y - window.series.priceToCoordinate(open)) },
      { price: high, distance: Math.abs(y - window.series.priceToCoordinate(high)) },
      { price: low, distance: Math.abs(y - window.series.priceToCoordinate(low)) },
      { price: close, distance: Math.abs(y - window.series.priceToCoordinate(close)) },
    ]

    const result = list.reduce((a, b) => (b.distance < a.distance ? b : a))

    if (window.magnet && result.distance <= 20) {
      window.chart.setCrosshairPosition(result.price, time, window.series)
    }

    class TooltipView {
      constructor(open, high, low, close, volume, font, textColor, locale, priceFormatter, upColor, downColor) {
        this.open = open
        this.high = high
        this.low = low
        this.close = close
        this.volume = volume
        this.font = font
        this.textColor = textColor
        this.locale = locale
        this.priceFormatter = priceFormatter
        this.upColor = upColor
        this.downColor = downColor
      }

      draw(target) {
        target.useMediaCoordinateSpace(scope => {
          const context = scope.context;
          const open = this.priceFormatter(this.open);
          const high = this.priceFormatter(this.high);
          const low = this.priceFormatter(this.low);
          const close = this.priceFormatter(this.close);
          const volume = this.priceFormatter(this.volume);

          const numberColor = this.close > this.open ? this.upColor : this.downColor;

          const textParts = this.locale === "zh-CN"
            ? [
              `开=`, open,
              ` 高=`, high,
              ` 低=`, low,
              ` 收=`, close,
              ` 量=`, volume
            ]
            : [
              `Open=`, open,
              ` High=`, high,
              ` Low=`, low,
              ` Close=`, close,
              ` Volume=`, volume
            ];

          context.font = this.font;

          let x = 10;
          const y = 20;

          textParts.forEach((part, index) => {
            context.fillStyle = index % 2 === 0 ? this.textColor : numberColor;
            context.fillText(part, x, y);
            x += context.measureText(part).width;
          });
        })
      }
    }

    window.vm.updateLayer("tooltip", new TooltipView(open, high, low, close, volume, "600 13px JetBrains Mono", window.chart.options().layout.textColor, locale, window.chart.options().localization.priceFormatter, buyColor, sellColor))
  })

  window.chart.subscribeClick(({ hoveredObjectId }) => {
    if (hoveredObjectId) {
      const record = document.getElementById("record_" + hoveredObjectId)

      if (record) {
        const log = record.closest(".log")
        const card = record.closest(".position-card")
        const color = getComputedStyle(document.querySelector("body")).getPropertyValue('--highlight-color')

        log.style.display = "block"

        document.querySelector("button[data-tab='history']").click()

        card.scrollIntoView({
          behavior: 'smooth',
          block: 'start',
          inline: 'nearest'
        })

        const observer = new IntersectionObserver((entries) => {
          entries.forEach(entry => {
            if (entry.isIntersecting) {
              let flag = false

              const interval = setInterval(() => {
                if (flag) {
                  record.style.backgroundColor = color
                } else {
                  record.style.backgroundColor = ""
                }
                flag = !flag
              }, 100)

              setTimeout(() => {
                clearInterval(interval)
                record.style.backgroundColor = ""
              }, 1000)

              observer.disconnect()
            }
          })
        }, { threshold: 0.5 })

        observer.observe(card)
      }
    }
  })

  if (window.dataSource && window.series) {
    const markerSymbol = window.dataSource.metadata.symbol
    window.series.setMarkers([])
    window.series.setMarkers(window.historyPositionList.filter(v => v.symbol == markerSymbol).flatMap(v => createMaker(v)))
  }
}

function getTimeRange(time, level) {
  if (typeof time !== 'number' || isNaN(time)) {
    throw new Error('亲爱的，时间戳不对劲……让我重新温柔地抱住你');
  }
  const dt = new Date(time);
  if (isNaN(dt.getTime())) {
    throw new Error('哦……时间迷失了，让我用身体帮你找回来');
  }

  switch (level) {
    case '1m': {
      const start = new Date(dt);
      start.setUTCSeconds(0);
      start.setUTCMilliseconds(0);
      const next = new Date(start.getTime() + 60000);
      return [start.getTime(), next.getTime()];
    }
    case '3m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 3) * 60000);
      const next = new Date(start.getTime() + 180000);
      return [start.getTime(), next.getTime()];
    }
    case '5m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 5) * 60000);
      const next = new Date(start.getTime() + 300000);
      return [start.getTime(), next.getTime()];
    }
    case '15m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 15) * 60000);
      const next = new Date(start.getTime() + 900000);
      return [start.getTime(), next.getTime()];
    }
    case '30m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 30) * 60000);
      const next = new Date(start.getTime() + 1800000);
      return [start.getTime(), next.getTime()];
    }
    case '1h': {
      const start = new Date(dt);
      start.setUTCMinutes(0);
      start.setUTCSeconds(0);
      start.setUTCMilliseconds(0);
      const next = new Date(start.getTime() + 3600000);
      return [start.getTime(), next.getTime()];
    }
    case '2h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 2) * 3600000);
      const next = new Date(start.getTime() + 7200000);
      return [start.getTime(), next.getTime()];
    }
    case '4h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 4) * 3600000);
      const next = new Date(start.getTime() + 14400000);
      return [start.getTime(), next.getTime()];
    }
    case '6h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 6) * 3600000);
      const next = new Date(start.getTime() + 21600000);
      return [start.getTime(), next.getTime()];
    }
    case '12h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 12) * 3600000);
      const next = new Date(start.getTime() + 43200000);
      return [start.getTime(), next.getTime()];
    }
    case '1d': {
      const start = new Date(dt);
      start.setUTCHours(0, 0, 0, 0);
      const next = new Date(start.getTime() + 86400000);
      return [start.getTime(), next.getTime()];
    }
    case '3d': {
      const dayStart = new Date(dt);
      dayStart.setUTCHours(0, 0, 0, 0);
      const ceStart = new Date(Date.UTC(1, 0, 1));
      const msDiff = dayStart.getTime() - ceStart.getTime();
      const days = Math.floor(msDiff / 86400000) + 1;
      const startOrdinal = Math.floor(days / 3) * 3;
      const startMs = ceStart.getTime() + (startOrdinal - 1) * 86400000;
      const start = new Date(startMs);
      const next = new Date(startMs + 259200000);
      return [start.getTime(), next.getTime()];
    }
    case '1w': {
      const start = new Date(dt);
      start.setUTCHours(0, 0, 0, 0);
      const weekday = start.getUTCDay();
      const daysToSubtract = (weekday + 6) % 7;
      start.setUTCDate(start.getUTCDate() - daysToSubtract);
      const next = new Date(start.getTime() + 604800000);
      return [start.getTime(), next.getTime()];
    }
    case '1mo': {
      const start = new Date(dt);
      start.setUTCHours(0, 0, 0, 0);
      start.setUTCDate(1);
      const next = new Date(start.getTime());
      next.setUTCMonth(next.getUTCMonth() + 1);
      next.setUTCHours(0, 0, 0, 0);
      return [start.getTime(), next.getTime()];
    }
    default:
      throw new Error('宝贝，level不对……告诉我你想要哪种节奏，我马上为你调整到最销魂的频率~');
  }
}

function createMaker(position) {
  const buy = locale == "zh-CN" ? "买" : "Buy"
  const sell = locale == "zh-CN" ? "卖" : "Sell"
  const buyColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--buy-color')
  const sellColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--sell-color')

  return position.log.map(v => {
    if (v.side == "Buy") {
      return {
        id: v.id,
        time: getTimeRange(v.time, window.dataSource.metadata.level)[0],
        position: 'belowBar',
        color: buyColor,
        shape: 'arrowUp',
        text: buy,
      }
    } else {
      return {
        id: v.id,
        time: getTimeRange(v.time, window.dataSource.metadata.level)[0],
        position: 'aboveBar',
        color: sellColor,
        shape: 'arrowDown',
        text: sell,
      }
    }
  })
}

const [chart, series] = initChart()

if (dataSourceList.length != 0) {
  window.dataSource = dataSourceList[0]
  applyOptions()
  series.setData(window.dataSource.data)

  // 设置成交量数据
  if (window.volumeSeries && window.dataSource.data && window.showVolume) {
    const buyColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--buy-color')
    const sellColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--sell-color')
    const volumeData = window.dataSource.data.map(item => ({
      time: item.time,
      value: item.volume || 0,
      color: item.close > item.open ? buyColor : sellColor
    }))

    window.volumeSeries.setData(volumeData)
  }

  chart.timeScale().fitContent()
}

//-----------------------------------------------------------------------
//-----------------------------------------------------------------------
//-----------------------------------------------------------------------

function createPositionCard(position, priceFormatter) {
  const s = (a, b) => (locale == "zh-CN" ? a : b)
  const card = document.createElement("div")
  const isLiquidation = position.log[position.log.length - 1].kind == 'Liquidation'
  const isFullClose = position.max_quantity == position.close_quantity
  const isBuy = position.side == 'Buy'
  const openAvgPrice = Number(position.open_avg_price)
  const maxQuantity = Number(position.max_quantity)
  const leverage = Number(position.leverage)
  const profit = Number(position.profit)
  const initialMargin = Number.isFinite(openAvgPrice) && Number.isFinite(maxQuantity) && Number.isFinite(leverage) && leverage > 0
    ? (openAvgPrice * maxQuantity) / leverage
    : null
  const closeReturnPct = Number.isFinite(profit) && Number.isFinite(initialMargin) && initialMargin > 0
    ? (profit / initialMargin) * 100
    : null
  const closeReturnPctText = closeReturnPct == null ? "-" : `${closeReturnPct >= 0 ? "+" : ""}${closeReturnPct.toFixed(2)}%`
  const closeReturnPctClass = closeReturnPct == null
    ? ""
    : closeReturnPct >= 0
      ? "position-card-value-profit-positive"
      : "position-card-value-profit-negative"

  const statusText = isLiquidation
    ? s("强平", "Liquidation")
    : isFullClose
      ? s("完全平仓", "Full Close")
      : s("部分平仓", "Partial Close")

  card.className = "order-card position-card"

  card.innerHTML = `
      <div class="order-card-head">
        <div class="order-card-title position-card-title">${position.symbol}</div>
        <div class="order-card-tags">
          <span class="order-card-tag ${isBuy ? 'buy' : 'sell'}">${statusText}</span>
          <span class="order-card-tag ${isBuy ? 'buy' : 'sell'}"">${position.leverage}x</span>
          <span class="order-card-tag ${isBuy ? 'buy' : 'sell'}"">${s("逐仓", "Isolated")}</span>
          <span class="order-card-tag ${isBuy ? 'buy' : 'sell'}">${isBuy ? s("买", "Buy") : s("卖", "Sell")}</span>
        </div>
      </div>
      <div class="order-card-grid position-card-grid">
        <div class="order-card-item">
          <div class="order-card-label">${s("开仓均价", "Entry Price")}</div>
          <div class="order-card-value">${priceFormatter(position.open_avg_price)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("最大持仓量", "Max Position Size")}</div>
          <div class="order-card-value">${position.max_quantity}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("平仓均价", "Exit Price")}</div>
          <div class="order-card-value">${priceFormatter(position.close_avg_price)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("平仓量", "Close Quantity")}</div>
          <div class="order-card-value">${position.close_quantity}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("净盈亏", "Net PnL")}</div>
          <div class="order-card-value ${position.total_profit >= 0 ? 'position-card-value-profit-positive' : 'position-card-value-profit-negative'}">${position.total_profit >= 0 ? '+' : ''}${position.total_profit}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("收益率", "Rate of Return")}%</div>
          <div class="order-card-value ${closeReturnPctClass}">${closeReturnPctText}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("毛盈亏", "Gross PnL")}</div>
          <div class="order-card-value ${position.profit >= 0 ? 'position-card-value-profit-positive' : 'position-card-value-profit-negative'}">${position.profit >= 0 ? '+' : ''}${position.profit}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("手续费", "Fee")}</div>
          <div class="order-card-value position-card-value-profit-negative">-${position.fee}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("开仓时间", "Entry Time")}</div>
          <div class="order-card-value">${new Date(position.open_time).toLocaleString()}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("平仓时间", "Exit Time")}</div>
          <div class="order-card-value">${new Date(position.close_time).toLocaleString()}</div>
        </div>
      </div>
      <div class="log">
        ${position.log.map(log => `
          <div class="position-card-section ${log.side == 'Buy' ? 'position-card-side-buy' : 'position-card-side-sell'}" id="record_${log.id}">
            <div>${new Date(log.time).toLocaleString()}</div>
            <div>${priceFormatter(log.price)}</div>
            <div>${log.quantity}</div>
            <div>
              ${log.kind == 'Liquidation' ? s("强平", "Liquidation") : log.side == 'Buy' ? s("买", "Buy") : s("卖", "Sell")}
            </div>
          </div>
        `).join('')}
      </div>
    `

  card.addEventListener("click", (event) => {
    const log = card.querySelector(".log")

    if (!event.target.closest('.log')) {
      log.style.display = log.style.display != "block" ? "block" : "none"
    } else {
      log.style.display = "block"
    }
  })

  return card
}

function addPositionCard(container, position, callback) {
  if (typeof container == "string") {
    container = document.querySelector(container)
  }

  const card = createPositionCard(position, window.chart.options().localization.priceFormatter)

  card.querySelector(".log").addEventListener("click", (event) => {
    const target = event.target.closest(".position-card-section")

    if (target) {
      const index = Array.from(card.querySelectorAll(".log .position-card-section")).indexOf(target)

      callback(index, position)
    }
  })

  if (container.children.length == 0) {
    card.querySelector(".log").style.display = "block"
  }

  container.appendChild(card)
}

function initHistoryTabs() {
  const tabbar = document.querySelector("#history-tabbar")
  const isZh = (locale || "").toLowerCase().startsWith("zh")

  if (!tabbar) {
    return
  }

  const tabs = tabbar.querySelectorAll(".history-tab")
  const views = document.querySelectorAll(".history-view")

  tabs.forEach(tab => {
    const tabKey = tab.getAttribute("data-tab")

    if (tabKey == "summary") {
      tab.textContent = isZh ? "总结" : "Summary"
    }

    if (tabKey == "history") {
      tab.textContent = isZh ? "历史仓位" : "History Position"
    }

    if (tabKey == "order") {
      tab.textContent = isZh ? "历史订单" : "History Order"
    }
  })

  tabs.forEach(tab => {
    tab.addEventListener("click", () => {
      const selected = tab.getAttribute("data-tab")

      tabs.forEach(item => item.classList.remove("active"))
      views.forEach(view => view.classList.remove("active"))

      tab.classList.add("active")

      const viewId = {
        summary: "summary-container",
        history: "history-position-container",
        order: "history-order-container",
      }[selected] || "history-position-container"

      const targetView = document.querySelector(`#${viewId}`)

      if (targetView) {
        targetView.classList.add("active")
      }
    })
  })
}

function renderSummary(symbol) {
  const container = document.querySelector("#summary-container")
  const s = (a, b) => (locale == "zh-CN" ? a : b)

  if (!container) {
    return
  }

  const list = symbol
    ? window.historyPositionList.filter(v => v.symbol == symbol)
    : window.historyPositionList

  if (list.length == 0) {
    container.innerHTML = `<div class="summary-note">${s("暂无可统计的历史仓位", "No historical positions to summarize")}</div>`
    return
  }

  const totalTrades = list.length
  const totalProfit = list.reduce((acc, v) => acc + Number(v.total_profit || 0), 0)
  const totalFee = list.reduce((acc, v) => acc + Number(v.fee || 0), 0)
  const winTrades = list.filter(v => Number(v.total_profit || 0) > 0).length
  const lossTrades = list.filter(v => Number(v.total_profit || 0) < 0).length
  const winRate = totalTrades == 0 ? 0 : winTrades / totalTrades * 100
  const avgProfit = totalTrades == 0 ? 0 : totalProfit / totalTrades
  const netProfits = list.map(v => Number(v.total_profit || 0))
  const netGrossProfit = netProfits.filter(v => v > 0).reduce((acc, v) => acc + v, 0)
  const netGrossLossAbs = Math.abs(netProfits.filter(v => v < 0).reduce((acc, v) => acc + v, 0))
  const profitLossRatio = netGrossLossAbs == 0 ? null : netGrossProfit / netGrossLossAbs
  const grossPnLList = list.map(v => Number(v.profit || 0))
  const grossProfit = grossPnLList.filter(v => v > 0).reduce((acc, v) => acc + v, 0)
  const grossLossAbs = Math.abs(grossPnLList.filter(v => v < 0).reduce((acc, v) => acc + v, 0))
  const bestTrade = Math.max(...netProfits)
  const worstTrade = Math.min(...netProfits)

  const fx = value => {
    const sign = value >= 0 ? "+" : ""
    return `${sign}${value.toFixed(2)}`
  }

  const valueClass = value => value > 0 ? "positive" : value < 0 ? "negative" : ""

  container.innerHTML = `
    <div class="summary-grid">
      <div class="summary-card">
        <div class="summary-card-label">${s("总交易数", "Total Trades")}</div>
        <div class="summary-card-value">${totalTrades}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("胜率", "Win Rate")}</div>
        <div class="summary-card-value">${winRate.toFixed(1)}%</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("盈利笔数", "Winning Trades")}</div>
        <div class="summary-card-value">${winTrades}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("亏损笔数", "Losing Trades")}</div>
        <div class="summary-card-value">${lossTrades}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("总收益", "Net PnL")}</div>
        <div class="summary-card-value ${valueClass(totalProfit)}">${fx(totalProfit)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("盈亏比", "Profit Factor")}</div>
        <div class="summary-card-value ${profitLossRatio == null ? "" : profitLossRatio >= 1 ? "positive" : "negative"}">${profitLossRatio == null ? "∞" : profitLossRatio.toFixed(2)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("净盈利", "Total Net Profit")}</div>
        <div class="summary-card-value positive">${fx(netGrossProfit)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("净亏损", "Total Net Loss")}</div>
        <div class="summary-card-value negative">-${netGrossLossAbs.toFixed(2)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("毛盈利", "Gross Profit")}</div>
        <div class="summary-card-value positive">${fx(grossProfit)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("毛亏损", "Gross Loss")}</div>
        <div class="summary-card-value negative">-${grossLossAbs.toFixed(2)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("手续费", "Fee")}</div>
        <div class="summary-card-value negative">-${totalFee.toFixed(2)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("平均单笔收益", "Average PnL per Trade")}</div>
        <div class="summary-card-value ${valueClass(avgProfit)}">${fx(avgProfit)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("最佳单笔", "Best Trade")}</div>
        <div class="summary-card-value ${valueClass(bestTrade)}">${fx(bestTrade)}</div>
      </div>
      <div class="summary-card">
        <div class="summary-card-label">${s("最差单笔", "Worst Trade")}</div>
        <div class="summary-card-value ${valueClass(worstTrade)}">${fx(worstTrade)}</div>
      </div>
    </div>
  `
}

function renderHistoryPositionList() {
  const container = document.querySelector("#history-position-container")

  if (!container) {
    return
  }

  container.innerHTML = ""

  for (const position of window.historyPositionList) {
    addPositionCard(container, position, (index, position) => {
      scrollChartToTime(position.log[index].time)
    })
  }

  if (window.historyPositionList.length == 0) {
    container.innerHTML = `<div class="summary-note">${s("暂无历史仓位", "No history positions to display")}</div>`
  }
}

function scrollChartToTime(time) {
  if (!window.dataSource || !window.dataSource.data || window.dataSource.data.length == 0) {
    return
  }

  const current = Number(getTimeRange(time, window.dataSource.metadata.level)[0])

  if (!Number.isFinite(current)) {
    return
  }

  window.series.priceScale().applyOptions({ autoScale: true })
  window.volumeSeries.priceScale().applyOptions({ autoScale: true })

  const visibleRange = chart.timeScale().getVisibleRange()

  if (!visibleRange || !Number.isFinite(visibleRange.from) || !Number.isFinite(visibleRange.to)) {
    return
  }

  const distance = Math.abs(visibleRange.to - visibleRange.from) / 2
  const dataMinTime = window.dataSource.data[0]?.time || visibleRange.from

  let from = current - distance
  let to = current + distance

  if (from < dataMinTime) {
    const compensation = dataMinTime - from
    to = to - compensation
    from = dataMinTime
  }

  chart.timeScale().setVisibleRange({
    from: from,
    to: to,
  })

  flashVerticalLineAtTime(current)
}

function clearFlashVerticalLine() {
  if (window.__flashLineTimer) {
    clearTimeout(window.__flashLineTimer)
    window.__flashLineTimer = null
  }

  if (window.chart) {
    const timeScale = chart.timeScale()

    if (window.__flashLineTimeRangeHandler) {
      timeScale.unsubscribeVisibleTimeRangeChange(window.__flashLineTimeRangeHandler)
      window.__flashLineTimeRangeHandler = null
    }

    if (window.__flashLineLogicalRangeHandler) {
      timeScale.unsubscribeVisibleLogicalRangeChange(window.__flashLineLogicalRangeHandler)
      window.__flashLineLogicalRangeHandler = null
    }
  }

  if (window.vm) {
    window.vm.clearLayer("flashLine")
  }

  window.__flashLineIgnoreRangeChangeUntil = 0
}

function flashVerticalLineAtTime(time) {
  if (!window.chart || !window.vm) {
    return
  }

  const current = Number(time)

  if (!Number.isFinite(current)) {
    return
  }

  clearFlashVerticalLine()

  class FlashVerticalLineView {
    constructor(time, color) {
      this.time = time
      this.color = color
    }

    draw(target) {
      const x = chart.timeScale().timeToCoordinate(this.time)

      if (!Number.isFinite(x)) {
        return
      }

      target.useMediaCoordinateSpace(scope => {
        const context = scope.context
        const pixelX = Math.round(x) + 0.5

        context.save()
        context.strokeStyle = this.color
        context.globalAlpha = 0.95
        context.lineWidth = 1
        context.beginPath()
        context.moveTo(pixelX, 0)
        context.lineTo(pixelX, scope.mediaSize.height)
        context.stroke()
        context.restore()
      })
    }
  }

  const lineColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--highlight-color').trim() || "#ff9800"

  window.vm.updateLayer("flashLine", new FlashVerticalLineView(current, lineColor))

  const destroyOnViewportChange = () => {
    if ((window.__flashLineIgnoreRangeChangeUntil || 0) > Date.now()) {
      return
    }

    clearFlashVerticalLine()
  }

  window.__flashLineTimeRangeHandler = destroyOnViewportChange
  window.__flashLineLogicalRangeHandler = destroyOnViewportChange

  const timeScale = chart.timeScale()
  // Ignore the immediate range-change events caused by our own setVisibleRange call.
  window.__flashLineIgnoreRangeChangeUntil = Date.now() + 120
  timeScale.subscribeVisibleTimeRangeChange(window.__flashLineTimeRangeHandler)
  timeScale.subscribeVisibleLogicalRangeChange(window.__flashLineLogicalRangeHandler)

  window.__flashLineTimer = setTimeout(() => {
    clearFlashVerticalLine()
  }, 1000)
}

function renderOrderList(symbol) {
  const container = document.querySelector("#history-order-container")
  const isZh = (locale || "").toLowerCase().startsWith("zh")
  const s = (a, b) => (isZh ? a : b)

  if (!container) {
    return
  }

  const list = symbol
    ? window.historyOrderList.filter(v => v.symbol == symbol)
    : window.historyOrderList

  container.innerHTML = ""

  const enumText = (value) => {
    if (value == undefined || value == null || value === "") {
      return "-"
    }

    return String(value)
  }

  const priceText = (value) => {
    const n = Number(value)

    if (!Number.isFinite(n)) {
      return "-"
    }

    return window.chart.options().localization.priceFormatter(n)
  }

  const qtyText = (value) => {
    const n = Number(value)

    if (!Number.isFinite(n)) {
      return "-"
    }

    const tick_size = window.dataSource.metadata.min_size;
    const snapped = Math.round(value / tick_size) * tick_size;
    const precision = (tick_size.toString().split('.')[1] || '').length;

    return snapped.toFixed(precision);
  }


  const kindText = (value) => {
    const key = enumText(value)

    return {
      Trigger: s("触发单", "Trigger"),
      Marker: s("市价单", "Market"),
      Limit: s("限价单", "Limit"),
      Liquidation: s("强平单", "Liquidation"),
      ADL: s("自动减仓", "ADL"),
    }[key] || key
  }

  const statusText = (value, kind) => {
    const key = enumText(value)

    return {
      Submitted: s("已提交", "Submitted"),
      PartiallyFilled: s("部分成交", "Partially Filled"),
      Filled: kind == "Trigger" ? s("已触发", "Triggered") : s("已成交", "Filled"),
      Canceled: s("已取消", "Canceled"),
      Rejected: s("已拒绝", "Rejected"),
    }[key] || key
  }

  for (const order of list) {
    const side = enumText(order.side)
    const isBuy = side == "Buy"
    const createTime = Number(order.create_time)
    const updateTime = Number(order.update_time)

    const card = document.createElement("div")
    card.className = "order-card"
    card.innerHTML = `
      <div class="order-card-head">
        <div class="order-card-title">${enumText(order.id)}</div>
        <div class="order-card-tags">
          ${order.reduce_only ? `<span class="order-card-tag ${isBuy ? "buy" : "sell"}">${s("只减仓", "Reduce Only")}</span>` : ""}
          <span class="order-card-tag ${isBuy ? "buy" : "sell"}">${statusText(order.status, order.kind)}</span>
          <span class="order-card-tag ${isBuy ? "buy" : "sell"}">${kindText(order.kind)}</span>
          <span class="order-card-tag ${isBuy ? "buy" : "sell"}">${side == "Buy" ? s("买", "Buy") : side == "Sell" ? s("卖", "Sell") : side}</span>
        </div>
      </div>
      <div class="order-card-grid">
        <div class="order-card-item">
          <div class="order-card-label">${s("交易对", "Symbol")}</div>
          <div class="order-card-value">${enumText(order.symbol)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("触发价", "Trigger Price")}</div>
          <div class="order-card-value">${priceText(order.trigger_price)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("委托价", "Order Price")}</div>
          <div class="order-card-value">${priceText(order.price)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("数量", "Quantity")}</div>
          <div class="order-card-value">${qtyText(order.quantity)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("成交均价", "Average Fill")}</div>
          <div class="order-card-value">${priceText(order.avg_price)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("累计成交", "Cumulative Qty")}</div>
          <div class="order-card-value">${qtyText(order.cumulative_quantity)}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("创建时间", "Create Time")}</div>
          <div class="order-card-value">${Number.isFinite(createTime) ? new Date(createTime).toLocaleString() : "-"}</div>
        </div>
        <div class="order-card-item">
          <div class="order-card-label">${s("更新时间", "Update Time")}</div>
          <div class="order-card-value">${Number.isFinite(updateTime) ? new Date(updateTime).toLocaleString() : "-"}</div>
        </div>
      </div>
    `

    card.addEventListener("click", () => {
      scrollChartToTime(order.update_time)
    })

    container.appendChild(card)
  }

  if (list.length == 0) {
    container.innerHTML = `<div class="summary-note">${s("暂无历史订单", "No history orders to display")}</div>`
  }
}

export function updateHistoryPosition() {
  renderHistoryPositionList(window.historyPositionList)

  const selectedSymbol = document.querySelector("#symbol-button")?.innerText
  const summarySymbol = selectedSymbol || symbolList[0]

  if (summarySymbol != undefined) {
    renderSummary(summarySymbol)
  }

  if (window.dataSource && window.series) {
    const markerSymbol = window.dataSource.metadata.symbol
    window.series.setMarkers(window.historyPositionList.filter(v => v.symbol == markerSymbol).flatMap(v => createMaker(v)))
  }

  return window.historyPositionList
}

export function updateOrderList() {
  const selectedSymbol = document.querySelector("#symbol-button")?.innerText
  const orderSymbol = selectedSymbol || symbolList[0]

  renderOrderList(orderSymbol)
}


initHistoryTabs()
renderHistoryPositionList(window.historyPositionList)

//-----------------------------------------------------------------------
//-----------------------------------------------------------------------
//-----------------------------------------------------------------------

const symbolList = [...new Set(dataSourceList.map(v => v.metadata.symbol))]
const levelList = [...new Set(dataSourceList.map(v => v.metadata.level))]
const themeList = locale == "zh-CN" ? ["暗色", "浅色"] : ["Dark", "Light"]

if (dataSourceList.length != 0) {
  document.querySelector("#symbol-button").innerHTML = symbolList[0]
  document.querySelector("#level-button").innerHTML = levelList[0]
}

renderSummary(symbolList[0])
renderOrderList(symbolList[0])

document.querySelector("#theme-button").innerText = theme == "dark" ? themeList[0] : themeList[1]

function initSwitches() {
  const magnetSwitch = document.querySelector("#magnet-switch")
  const volumeSwitch = document.querySelector("#volume-switch")

  if (magnetSwitch) {
    if (window.magnet) {
      magnetSwitch.classList.add("active")
    } else {
      magnetSwitch.classList.remove("active")
    }
  }

  if (volumeSwitch) {
    if (window.showVolume) {
      volumeSwitch.classList.add("active")
    } else {
      volumeSwitch.classList.remove("active")
    }

  }

  if (magnetSwitch) {
    magnetSwitch.addEventListener("click", () => {
      window.magnet = !window.magnet
      localStorage.setItem("magnet", window.magnet)

      if (window.magnet) {
        magnetSwitch.classList.add("active")
      } else {
        magnetSwitch.classList.remove("active")
      }
    })
  }

  if (volumeSwitch) {
    volumeSwitch.addEventListener("click", () => {
      window.showVolume = !window.showVolume
      localStorage.setItem("showVolume", window.showVolume)

      if (window.showVolume) {
        volumeSwitch.classList.add("active")
      } else {
        volumeSwitch.classList.remove("active")
      }

      if (window.showVolume) {
        const buyColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--buy-color')
        const sellColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--sell-color')
        const volumeData = window.dataSource.data.map(item => ({
          time: item.time,
          value: item.volume || 0,
          color: item.close > item.open ? buyColor : sellColor
        }))


        window.volumeSeries.setData(volumeData)
        window.volumeSeries.applyOptions({ visible: true })
      } else {
        window.volumeSeries.applyOptions({ visible: false })
      }
    })
  }
}

initSwitches()

function createDropdownList(id, list, callback) {
  const dropdown = document.querySelector(id);

  dropdown.innerHTML = "";

  list.forEach(item => {
    const button = document.createElement("button");

    button.innerText = item;

    button.addEventListener("click", function () {
      callback(item);
    });

    dropdown.appendChild(button);
  })
}

createDropdownList("#symbol-dropdown", symbolList, (value) => {
  document.querySelector("#symbol-button").innerText = value;
  document.querySelector("#symbol-dropdown").classList.remove("show");
  renderSummary(value)
  renderOrderList(value)

  if (window.dataSourceList.length == 0) {
    return
  }

  const level = document.querySelector("#level-button").innerText

  window.dataSource = dataSourceList.find(v => levelText ? v.metadata.symbol == value && v.metadata.level == level : v.metadata.symbol == value)
  window.series.setMarkers([])
  window.series.setMarkers(window.historyPositionList.filter(v => v.symbol == value).flatMap(v => createMaker(v)))
  window.series.setData(window.dataSource.data)

  if (window.showVolume) {
    const buyColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--buy-color')
    const sellColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--sell-color')
    const volumeData = window.dataSource.data.map(item => ({
      time: item.time,
      value: item.volume || 0,
      color: item.close > item.open ? buyColor : sellColor
    }))

    window.volumeSeries.setData(volumeData)
    window.volumeSeries.applyOptions({ visible: true })
  } else {
    window.volumeSeries.applyOptions({ visible: false })
  }
});

createDropdownList("#level-dropdown", levelList, (value) => {
  document.querySelector("#level-button").innerText = value;
  document.querySelector("#level-dropdown").classList.remove("show");

  if (dataSourceList.length == 0) {
    return
  }

  const symbol = document.querySelector("#symbol-button").innerText

  const nextDataSource = dataSourceList.find(v => v.metadata.symbol == symbol && v.metadata.level == value)

  if (!nextDataSource) {
    return
  }

  window.dataSource = nextDataSource
  series.setData(nextDataSource.data)
  series.setMarkers([])
  series.setMarkers(window.historyPositionList.filter(v => v.symbol == symbol).flatMap(v => createMaker(v)))

  if (window.showVolume) {
    const buyColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--buy-color')
    const sellColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--sell-color')
    const volumeData = window.dataSource.data.map(item => ({
      time: item.time,
      value: item.volume || 0,
      color: item.close > item.open ? buyColor : sellColor
    }))

    window.volumeSeries.setData(volumeData)
    window.volumeSeries.applyOptions({ visible: true })
  } else {
    window.volumeSeries.applyOptions({ visible: false })
  }
});

createDropdownList("#theme-dropdown", themeList, (value) => {
  document.querySelector("#theme-button").innerText = value
  document.querySelector("#theme-dropdown").classList.remove("show")
  window.theme = value == themeList[0] ? "dark" : "light"
  localStorage.setItem("theme", window.theme)

  applyOptions()

  if (window.showVolume) {
    const buyColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--buy-color')
    const sellColor = getComputedStyle(document.querySelector("body")).getPropertyValue('--sell-color')
    const volumeData = dataSource.data.map(item => ({
      time: item.time,
      value: item.volume || 0,
      color: item.close > item.open ? buyColor : sellColor
    }))

    window.volumeSeries.setData(volumeData)
    window.volumeSeries.applyOptions({ visible: true })
  } else {
    window.volumeSeries.applyOptions({ visible: false })
  }
});

function toggleDropdown(id) {
  const dropdowns = document.querySelectorAll(".dropdown-content")
  dropdowns.forEach(dropdown => dropdown.classList.remove("show"))
  document.querySelector(id).classList.toggle("show")
}

document.querySelector("#symbol-button").addEventListener("click", () => {
  toggleDropdown("#symbol-dropdown");
});

document.querySelector("#level-button").addEventListener("click", () => {
  toggleDropdown("#level-dropdown");
});

document.querySelector("#theme-button").addEventListener("click", () => {
  toggleDropdown("#theme-dropdown");
});

window.addEventListener("click", function (event) {
  if (!event.target.matches('.dropdown-button')) {
    const dropdowns = document.querySelectorAll(".dropdown-content");

    dropdowns.forEach(openDropdown => openDropdown.classList.remove('show'));
  }
});
