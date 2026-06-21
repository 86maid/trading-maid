import React, { useRef } from 'react';
import { useChart } from '../hooks/useChart';
import { useTradingData } from '../context/TradingDataContext';

export default function ChartPanel({ onChartReady, historyPanelRef }) {
  const containerRef = useRef(null);
  const {
    currentDataSource,
    theme,
    magnet,
    showVolume,
    locale,
    historyPositionList,
  } = useTradingData();

  // Track active blink state for cleanup across rapid clicks
  const blinkRef = useRef({
    interval: null,
    timeout: null,
    scrollTimer: null,
    scrollCleanup: null,
  });

  // Handle marker click: switch to positions tab, expand the right card, scroll + blink
  const handleMarkerClick = React.useCallback(
    (hoveredObjectId) => {
      // 1. Clear any previous blink / scroll watcher
      if (blinkRef.current.interval) {
        clearInterval(blinkRef.current.interval);
        blinkRef.current.interval = null;
      }
      if (blinkRef.current.timeout) {
        clearTimeout(blinkRef.current.timeout);
        blinkRef.current.timeout = null;
      }
      if (blinkRef.current.scrollTimer) {
        clearTimeout(blinkRef.current.scrollTimer);
        blinkRef.current.scrollTimer = null;
      }
      if (blinkRef.current.scrollCleanup) {
        blinkRef.current.scrollCleanup();
        blinkRef.current.scrollCleanup = null;
      }

      // 2. Delegate scroll + expand to HistoryPanel's virtual list
      const historyPanel = historyPanelRef?.current;
      if (!historyPanel) return;

      historyPanel.scrollToRecord(hoveredObjectId, (recordId) => {
        if (!recordId) return;

        const record = document.getElementById('record_' + recordId);
        if (!record) return;

        // Manually scroll the trade-log inner container so the record
        // is centered — avoids scrollIntoView() which targets a random
        // scrollable ancestor and breaks our scroll-event listening.
        const logContainer = record.closest('[data-section="trade-log"]');
        if (logContainer) {
          const containerH = logContainer.clientHeight;
          const recordTop = record.offsetTop;
          logContainer.scrollTop = Math.max(0, recordTop - containerH / 2);
        }

        // Blink immediately — no waiting for scroll events
        const style = getComputedStyle(document.body);
        const color = style.getPropertyValue('--highlight-color').trim();

        if (blinkRef.current.interval) return; // already blinking
        let flag = false;
        blinkRef.current.interval = setInterval(() => {
          record.style.backgroundColor = flag ? color : '';
          flag = !flag;
        }, 100);
        blinkRef.current.timeout = setTimeout(() => {
          clearInterval(blinkRef.current.interval);
          blinkRef.current.interval = null;
          blinkRef.current.timeout = null;
          record.style.backgroundColor = '';
        }, 1000);
      });
    },
    [historyPanelRef]
  );

  const { scrollToTime } = useChart(
    containerRef,
    currentDataSource,
    theme,
    magnet,
    showVolume,
    locale,
    historyPositionList,
    handleMarkerClick
  );

  // Expose scrollToTime to parent
  React.useEffect(() => {
    if (onChartReady) {
      onChartReady(scrollToTime);
    }
  }, [scrollToTime, onChartReady]);

  return <div id="chart-container" ref={containerRef} />;
}
