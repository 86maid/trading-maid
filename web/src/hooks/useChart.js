import { useEffect, useRef, useCallback } from 'react';
import { createChart } from 'lightweight-charts';
import { timeFormatter, tickMarkFormatter, getTimeRange } from '../utils/timeUtils';
import { makePriceFormatter } from '../utils/priceUtils';
import { createMaker } from '../utils/markers';

// ViewManager from the original code — manages canvas overlay layers
class ViewManager {
  attached(primitive) {
    this.primitive = primitive;
    this.layers = {};
    this.list = [];
  }

  paneViews() {
    return this.list;
  }

  updateLayer(layerName, ...list) {
    this.layers[layerName] = list.map((v) => {
      v.renderer = () => v;
      return v;
    });
    this.list = Object.values(this.layers).flat();
    this.primitive.requestUpdate();
  }

  clearLayer(layerName) {
    if (!this.layers[layerName]) return;
    delete this.layers[layerName];
    this.list = Object.values(this.layers).flat();
    this.primitive.requestUpdate();
  }
}

// TooltipView for crosshair overlay
class TooltipView {
  constructor(open, high, low, close, volume, font, textColor, locale, priceFormatter, upColor, downColor) {
    this.open = open;
    this.high = high;
    this.low = low;
    this.close = close;
    this.volume = volume;
    this.font = font;
    this.textColor = textColor;
    this.locale = locale;
    this.priceFormatter = priceFormatter;
    this.upColor = upColor;
    this.downColor = downColor;
  }

  draw(target) {
    target.useMediaCoordinateSpace((scope) => {
      const context = scope.context;
      const open = this.priceFormatter(this.open);
      const high = this.priceFormatter(this.high);
      const low = this.priceFormatter(this.low);
      const close = this.priceFormatter(this.close);
      const vol = this.priceFormatter(this.volume);

      const numberColor = this.close > this.open ? this.upColor : this.downColor;

      const isZh = this.locale.startsWith('zh');
      const textParts = isZh
        ? ['开=', open, ' 高=', high, ' 低=', low, ' 收=', close, ' 量=', vol]
        : ['Open=', open, ' High=', high, ' Low=', low, ' Close=', close, ' Volume=', vol];

      context.font = this.font;

      let x = 10;
      const y = 20;

      textParts.forEach((part, index) => {
        context.fillStyle = index % 2 === 0 ? this.textColor : numberColor;
        context.fillText(part, x, y);
        x += context.measureText(part).width;
      });
    });
  }
}

// FlashVerticalLineView for the flash line effect
class FlashVerticalLineView {
  constructor(time, color, chart) {
    this.time = time;
    this.color = color;
    this.chart = chart;
  }

  draw(target) {
    const x = this.chart.timeScale().timeToCoordinate(this.time);
    if (!Number.isFinite(x)) return;

    target.useMediaCoordinateSpace((scope) => {
      const context = scope.context;
      const pixelX = Math.round(x) + 0.5;

      context.save();
      context.strokeStyle = this.color;
      context.globalAlpha = 0.95;
      context.lineWidth = 1;
      context.beginPath();
      context.moveTo(pixelX, 0);
      context.lineTo(pixelX, scope.mediaSize.height);
      context.stroke();
      context.restore();
    });
  }
}

export function useChart(
  containerRef,
  currentDataSource,
  theme,
  magnet,
  showVolume,
  locale,
  historyPositionList,
  onMarkerClick
) {
  const chartRef = useRef(null);
  const seriesRef = useRef(null);
  const volumeSeriesRef = useRef(null);
  const vmRef = useRef(null);

  // Refs for values used inside subscriptions (avoid re-subscribing)
  const magnetRef = useRef(magnet);
  const localeRef = useRef(locale);
  const currentDataSourceRef = useRef(currentDataSource);
  const flashTimerRef = useRef(null);
  const flashTimeRangeHandlerRef = useRef(null);
  const flashLogicalRangeHandlerRef = useRef(null);
  const flashIgnoreUntilRef = useRef(0);

  magnetRef.current = magnet;
  localeRef.current = locale;
  currentDataSourceRef.current = currentDataSource;

  // --- Apply chart options from theme ---
  const applyChartOptions = useCallback(() => {
    const chart = chartRef.current;
    const series = seriesRef.current;
    if (!chart || !series) return;
    if (!currentDataSource) return;

    const bodyStyle = getComputedStyle(document.body);

    const priceFormatterFn = makePriceFormatter(currentDataSource.metadata.tick_size);

    chart.applyOptions({
      layout: {
        background: { color: bodyStyle.getPropertyValue('--background-color') },
        textColor: bodyStyle.getPropertyValue('--label-color'),
      },
      grid: {
        vertLines: { color: bodyStyle.getPropertyValue('--grid-color') },
        horzLines: { color: bodyStyle.getPropertyValue('--grid-color') },
      },
      crosshair: {
        mode: 0,
        horzLine: {
          labelBackgroundColor: bodyStyle.getPropertyValue('--border-color'),
        },
        vertLine: {
          labelBackgroundColor: bodyStyle.getPropertyValue('--border-color'),
        },
      },
      timeScale: {
        borderColor: bodyStyle.getPropertyValue('--border-color'),
        tickMarkFormatter,
      },
      rightPriceScale: {
        borderColor: bodyStyle.getPropertyValue('--border-color'),
      },
      localization: {
        timeFormatter: (t) => timeFormatter(t, locale),
        priceFormatter: priceFormatterFn,
      },
      autoSize: true,
    });

    const buyColor = bodyStyle.getPropertyValue('--buy-color').trim();
    const sellColor = bodyStyle.getPropertyValue('--sell-color').trim();

    series.applyOptions({
      upColor: buyColor,
      downColor: sellColor,
      borderUpColor: buyColor,
      borderDownColor: sellColor,
      wickUpColor: buyColor,
      wickDownColor: sellColor,
      lastValueVisible: false,
      priceLineVisible: false,
      priceFormat: {
        type: 'custom',
        formatter: priceFormatterFn,
        minMove: currentDataSource.metadata.tick_size || 0.01,
      },
    });
  }, [currentDataSource, locale, theme]);

  // --- Initialize chart (once) ---
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const chart = createChart(container);
    const series = chart.addCandlestickSeries();
    const vm = new ViewManager();
    series.attachPrimitive(vm);

    const volumeSeries = chart.addHistogramSeries({
      lastValueVisible: false,
      priceLineVisible: false,
      priceFormat: { type: 'volume' },
      priceScaleId: 'volume',
    });
    volumeSeries.priceScale().applyOptions({
      scaleMargins: { top: 0.8, bottom: 0 },
    });

    chartRef.current = chart;
    seriesRef.current = series;
    volumeSeriesRef.current = volumeSeries;
    vmRef.current = vm;

    // Expose for devtools debugging
    window.chart = chart;
    window.series = series;
    window.volumeSeries = volumeSeries;
    window.vm = vm;

    return () => {
      chart.remove();
      chartRef.current = null;
      seriesRef.current = null;
      volumeSeriesRef.current = null;
      vmRef.current = null;
      window.chart = undefined;
      window.series = undefined;
      window.volumeSeries = undefined;
      window.vm = undefined;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // --- Apply options when theme or locale changes ---
  useEffect(() => {
    applyChartOptions();
  }, [applyChartOptions]);

  // --- Crosshair subscription (once, using refs) ---
  useEffect(() => {
    const chart = chartRef.current;
    const series = seriesRef.current;
    const vm = vmRef.current;
    if (!chart || !series || !vm) return;

    const unsubMove = chart.subscribeCrosshairMove(({ seriesData, point, time, logical, hoveredObjectId }) => {
      if (hoveredObjectId) {
        document.body.style.cursor = 'pointer';
      } else {
        document.body.style.cursor = 'default';
      }

      const k = seriesData.get(series);
      if (!k || !point || logical == null) return;

      const ds = currentDataSourceRef.current;
      if (!ds || !ds.data || !ds.data[logical]) return;

      const { open, high, low, close } = k;
      const volume = ds.data[logical].volume;

      // Magnet snapping
      if (magnetRef.current) {
        const y = point.y;
        const list = [
          { price: open, distance: Math.abs(y - series.priceToCoordinate(open)) },
          { price: high, distance: Math.abs(y - series.priceToCoordinate(high)) },
          { price: low, distance: Math.abs(y - series.priceToCoordinate(low)) },
          { price: close, distance: Math.abs(y - series.priceToCoordinate(close)) },
        ];
        const result = list.reduce((a, b) => (b.distance < a.distance ? b : a));
        if (result.distance <= 20) {
          chart.setCrosshairPosition(result.price, time, series);
        }
      }

      // Tooltip overlay
      const bodyStyle = getComputedStyle(document.body);
      const buyColor = bodyStyle.getPropertyValue('--buy-color').trim();
      const sellColor = bodyStyle.getPropertyValue('--sell-color').trim();
      const textColor = chart.options().layout.textColor;
      const priceFmt = chart.options().localization.priceFormatter;

      vm.updateLayer(
        'tooltip',
        new TooltipView(open, high, low, close, volume, '600 13px JetBrains Mono', textColor, localeRef.current, priceFmt, buyColor, sellColor)
      );
    });

    const unsubClick = chart.subscribeClick(({ hoveredObjectId }) => {
      if (hoveredObjectId && onMarkerClick) {
        onMarkerClick(hoveredObjectId);
      }
    });

    return () => {
      unsubMove();
      unsubClick();
    };
  }, [onMarkerClick]); // eslint-disable-line react-hooks/exhaustive-deps

  // --- Update chart data when dataSource changes ---
  useEffect(() => {
    const chart = chartRef.current;
    const series = seriesRef.current;
    const volumeSeries = volumeSeriesRef.current;
    if (!chart || !series || !currentDataSource) return;

    series.setData(currentDataSource.data);
    chart.timeScale().fitContent();

    // Update markers
    const markerSymbol = currentDataSource.metadata.symbol;
    const bodyStyle = getComputedStyle(document.body);
    const buyColor = bodyStyle.getPropertyValue('--buy-color').trim();
    const sellColor = bodyStyle.getPropertyValue('--sell-color').trim();
    const markers = (historyPositionList || [])
      .filter((v) => v.symbol === markerSymbol)
      .flatMap((v) => createMaker(v, locale, currentDataSource.metadata.level, buyColor, sellColor));
    series.setMarkers(markers);

    // Update volume
    updateVolume(volumeSeries, currentDataSource, showVolume, bodyStyle);
  }, [currentDataSource, showVolume, locale, historyPositionList]);

  // --- Update markers only ---
  useEffect(() => {
    const series = seriesRef.current;
    if (!series || !currentDataSource) return;

    const markerSymbol = currentDataSource.metadata.symbol;
    const bodyStyle = getComputedStyle(document.body);
    const buyColor = bodyStyle.getPropertyValue('--buy-color').trim();
    const sellColor = bodyStyle.getPropertyValue('--sell-color').trim();
    const markers = (historyPositionList || [])
      .filter((v) => v.symbol === markerSymbol)
      .flatMap((v) => createMaker(v, locale, currentDataSource.metadata.level, buyColor, sellColor));
    series.setMarkers(markers);
  }, [historyPositionList, currentDataSource, locale, theme]);

  // --- Show/hide volume series ---
  useEffect(() => {
    const volumeSeries = volumeSeriesRef.current;
    if (!volumeSeries || !currentDataSource) return;

    const bodyStyle = getComputedStyle(document.body);
    if (showVolume) {
      updateVolume(volumeSeries, currentDataSource, true, bodyStyle);
      volumeSeries.applyOptions({ visible: true });
    } else {
      volumeSeries.applyOptions({ visible: false });
    }
  }, [showVolume, currentDataSource, theme]);

  // --- Clear flash line on unmount ---
  useEffect(() => {
    return () => {
      if (flashTimerRef.current) {
        clearTimeout(flashTimerRef.current);
      }
    };
  }, []);

  // --- scrollToTime ---
  const scrollToTime = useCallback(
    (time) => {
      const chart = chartRef.current;
      const series = seriesRef.current;
      const vm = vmRef.current;
      const ds = currentDataSourceRef.current;
      if (!chart || !series || !vm || !ds) return;

      const current = Number(getTimeRange(time, ds.metadata.level)[0]);
      if (!Number.isFinite(current)) return;

      series.priceScale().applyOptions({ autoScale: true });

      const visibleRange = chart.timeScale().getVisibleRange();
      if (!visibleRange || !Number.isFinite(visibleRange.from) || !Number.isFinite(visibleRange.to)) return;

      const distance = Math.abs(visibleRange.to - visibleRange.from) / 2;
      const dataMinTime = ds.data[0]?.time || visibleRange.from;

      let from = current - distance;
      let to = current + distance;

      if (from < dataMinTime) {
        const compensation = dataMinTime - from;
        to = to - compensation;
        from = dataMinTime;
      }

      chart.timeScale().setVisibleRange({ from, to });

      // Flash vertical line
      clearFlashLine();
      const lineColor = getComputedStyle(document.body).getPropertyValue('--highlight-color').trim() || '#ff9800';

      vm.updateLayer('flashLine', new FlashVerticalLineView(current, lineColor, chart));

      const destroyOnViewportChange = () => {
        if ((flashIgnoreUntilRef.current || 0) > Date.now()) return;
        clearFlashLine();
      };

      flashTimeRangeHandlerRef.current = destroyOnViewportChange;
      flashLogicalRangeHandlerRef.current = destroyOnViewportChange;

      flashIgnoreUntilRef.current = Date.now() + 120;
      const timeScale = chart.timeScale();
      timeScale.subscribeVisibleTimeRangeChange(flashTimeRangeHandlerRef.current);
      timeScale.subscribeVisibleLogicalRangeChange(flashLogicalRangeHandlerRef.current);

      flashTimerRef.current = setTimeout(() => {
        clearFlashLine();
      }, 1000);
    },
    []
  );

  function clearFlashLine() {
    if (flashTimerRef.current) {
      clearTimeout(flashTimerRef.current);
      flashTimerRef.current = null;
    }

    const chart = chartRef.current;
    if (chart) {
      const timeScale = chart.timeScale();
      if (flashTimeRangeHandlerRef.current) {
        timeScale.unsubscribeVisibleTimeRangeChange(flashTimeRangeHandlerRef.current);
        flashTimeRangeHandlerRef.current = null;
      }
      if (flashLogicalRangeHandlerRef.current) {
        timeScale.unsubscribeVisibleLogicalRangeChange(flashLogicalRangeHandlerRef.current);
        flashLogicalRangeHandlerRef.current = null;
      }
    }

    const vm = vmRef.current;
    if (vm) {
      vm.clearLayer('flashLine');
    }
    flashIgnoreUntilRef.current = 0;
  }

  return { scrollToTime };
}

function updateVolume(volumeSeries, dataSource, show, bodyStyle) {
  if (!volumeSeries || !dataSource || !dataSource.data) return;
  if (!show) {
    volumeSeries.applyOptions({ visible: false });
    return;
  }
  const buyColor = bodyStyle.getPropertyValue('--buy-color');
  const sellColor = bodyStyle.getPropertyValue('--sell-color');
  const volumeData = dataSource.data.map((item) => ({
    time: item.time,
    value: Number(item.volume) || 0,
    color: item.close > item.open ? buyColor : sellColor,
  }));
  volumeSeries.setData(volumeData);
  volumeSeries.applyOptions({ visible: true });
}
