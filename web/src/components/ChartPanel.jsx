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

        // Record is now in view — blink it
        const record = document.getElementById('record_' + recordId);
        if (!record) return;

        const style = getComputedStyle(document.body);
        const color = style.getPropertyValue('--highlight-color').trim();

        // Scroll the record into view within its inner log container
        record.scrollIntoView({ behavior: 'smooth', block: 'center' });

        // Wait for inner scroll to settle, then blink
        const scrollContainer = record.closest('[data-section="trade-log"]');
        let settled = false;

        const startBlink = () => {
          if (settled) return;
          settled = true;
          if (blinkRef.current.interval) return;
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
        };

        if (scrollContainer) {
          const onScroll = () => {
            if (blinkRef.current.scrollTimer)
              clearTimeout(blinkRef.current.scrollTimer);
            blinkRef.current.scrollTimer = setTimeout(() => {
              scrollContainer.removeEventListener('scroll', onScroll);
              startBlink();
            }, 150);
          };
          scrollContainer.addEventListener('scroll', onScroll, { passive: true });
          blinkRef.current.scrollTimer = setTimeout(() => {
            scrollContainer.removeEventListener('scroll', onScroll);
            startBlink();
          }, 2000);
        } else {
          startBlink();
        }
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
