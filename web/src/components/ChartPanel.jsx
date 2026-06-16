import React, { useRef } from 'react';
import { useChart } from '../hooks/useChart';
import { useTradingData } from '../context/TradingDataContext';

export default function ChartPanel({ onChartReady }) {
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
  const blinkRef = useRef({ interval: null, timeout: null });

  // Handle marker click: switch to positions tab, expand the right card, scroll + blink
  const handleMarkerClick = React.useCallback(
    (hoveredObjectId) => {
      // 1. Clear any previous blink
      if (blinkRef.current.interval) {
        clearInterval(blinkRef.current.interval);
        blinkRef.current.interval = null;
      }
      if (blinkRef.current.timeout) {
        clearTimeout(blinkRef.current.timeout);
        blinkRef.current.timeout = null;
      }

      // 2. Switch to positions tab
      const posTab = document.querySelector('[data-node-key="positions"]');
      if (posTab) posTab.click();

      // 3. Find which position contains this log id
      const posList = window.historyPositionList || [];
      let targetPos = null;
      for (const pos of posList) {
        if (pos.log && pos.log.some((l) => l.id === hoveredObjectId)) {
          targetPos = pos;
          break;
        }
      }
      if (!targetPos) return;

      // 4. Wait for React to re-render the positions tab content
      requestAnimationFrame(() => {
        // Find the card by unique open_time
        const card = document.querySelector(
          `[data-open-time="${targetPos.open_time}"]`
        );
        if (!card) return;

        // Expand the card if collapsed (click it programmatically)
        const logEl = card.querySelector('[data-section="trade-log"]');
        if (!logEl || logEl.children.length === 0) {
          card.click();
        }

        // 5. Wait for React re-render + scroll, then blink
        requestAnimationFrame(() => {
          card.scrollIntoView({ behavior: 'smooth', block: 'start', inline: 'nearest' });

          const record = document.getElementById('record_' + hoveredObjectId);
          if (!record) return;

          // Show the log if hidden
          const log = record.closest('[data-section="trade-log"]');
          if (log) log.style.display = 'block';

          const style = getComputedStyle(document.body);
          const color = style.getPropertyValue('--highlight-color').trim();

          // Wait for smooth scroll to finish (~400ms), then blink
          blinkRef.current.timeout = setTimeout(() => {
            let flag = false;
            blinkRef.current.interval = setInterval(() => {
              if (flag) {
                record.style.backgroundColor = color;
              } else {
                record.style.backgroundColor = '';
              }
              flag = !flag;
            }, 100);
            blinkRef.current.timeout = setTimeout(() => {
              clearInterval(blinkRef.current.interval);
              blinkRef.current.interval = null;
              blinkRef.current.timeout = null;
              record.style.backgroundColor = '';
            }, 1000);
          }, 400);
        });
      });
    },
    []
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
