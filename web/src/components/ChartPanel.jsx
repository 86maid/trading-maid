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

          // Start the blink animation
          const startBlink = () => {
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
          };

          // Find the scrollable container that scrollIntoView will use
          let scrollContainer = card.parentElement;
          while (
            scrollContainer &&
            scrollContainer.scrollHeight <= scrollContainer.clientHeight
          ) {
            scrollContainer = scrollContainer.parentElement;
          }
          const scrollTarget =
            scrollContainer && scrollContainer !== document.documentElement
              ? scrollContainer
              : window;

          // Check if target is already in view (no scroll needed)
          const rect = record.getBoundingClientRect();
          const isInView =
            rect.top >= 0 &&
            rect.bottom <= window.innerHeight;

          if (isInView) {
            // Already visible, blink immediately
            startBlink();
          } else {
            // Wait for smooth scroll to actually finish, then blink
            const cleanupScrollWatch = () => {
              if (blinkRef.current.scrollTimer) {
                clearTimeout(blinkRef.current.scrollTimer);
                blinkRef.current.scrollTimer = null;
              }
              scrollTarget.removeEventListener('scroll', onScroll, {
                passive: true,
              });
            };
            blinkRef.current.scrollCleanup = cleanupScrollWatch;

            const onScroll = () => {
              // Debounce: restart the timer every time a scroll event fires
              if (blinkRef.current.scrollTimer)
                clearTimeout(blinkRef.current.scrollTimer);
              blinkRef.current.scrollTimer = setTimeout(() => {
                cleanupScrollWatch();
                blinkRef.current.scrollCleanup = null;
                startBlink();
              }, 150); // 150ms of no scrolling = scroll has ended
            };

            scrollTarget.addEventListener('scroll', onScroll, {
              passive: true,
            });

            // Safety fallback: if scroll events never settle, blink anyway
            blinkRef.current.scrollTimer = setTimeout(() => {
              cleanupScrollWatch();
              blinkRef.current.scrollCleanup = null;
              startBlink();
            }, 2000);
          }
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
