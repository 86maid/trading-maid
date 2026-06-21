import React, { useState, useRef, useCallback, useMemo, useImperativeHandle, forwardRef } from 'react';
import { VariableSizeList } from 'react-window';
import { Tabs, Empty } from 'antd';
import SummaryView from './SummaryView';
import PositionCard from './PositionCard';
import OrderCard from './OrderCard';
import { useTradingData } from '../context/TradingDataContext';

// ── Constants ──────────────────────────────────────────────────────────
const CARD_GAP = 12;
const OVERSCAN_COUNT = 3;
// Generous fallback estimates — replaced by real measurements after first paint.
const EST_POS = 280;
const EST_ORD = 220;

const HistoryPanel = forwardRef(function HistoryPanel(
  { scrollToTime, activeTab, onTabChange },
  ref
) {
  const { historyPositionList, historyOrderList, currentSymbol, isZh } =
    useTradingData();

  // ── Derived data (memoised, but also stored in refs for stable callbacks) ─
  const filteredPositions = useMemo(
    () => historyPositionList.filter((v) => v.symbol === currentSymbol),
    [historyPositionList, currentSymbol]
  );
  const filteredOrders = useMemo(
    () => historyOrderList.filter((v) => v.symbol === currentSymbol),
    [historyOrderList, currentSymbol]
  );

  // ── Refs — always hold the latest value without invalidating callbacks ──
  const filteredPositionsRef = useRef(filteredPositions);
  filteredPositionsRef.current = filteredPositions;
  const filteredOrdersRef = useRef(filteredOrders);
  filteredOrdersRef.current = filteredOrders;
  const currentSymbolRef = useRef(currentSymbol);
  currentSymbolRef.current = currentSymbol;
  const scrollToTimeRef = useRef(scrollToTime);
  scrollToTimeRef.current = scrollToTime;

  // ── Expanded state ──────────────────────────────────────────────────
  const [expandedPositions, setExpandedPositions] = useState(new Set());
  const expandedRef = useRef(expandedPositions);
  expandedRef.current = expandedPositions;

  // ── Measured height caches (content only, gap added by getItemSize) ──
  const posSizeCache = useRef({});
  const orderSizeCache = useRef({});

  // ── Batching: throttle resetAfterIndex ──────────────────────────────
  const posResetPending = useRef(false);
  const orderResetPending = useRef(false);

  const schedulePosReset = useCallback(() => {
    if (posResetPending.current) return;
    posResetPending.current = true;
    requestAnimationFrame(() => {
      posResetPending.current = false;
      posListRef.current?.resetAfterIndex(0);
    });
  }, []);

  const scheduleOrderReset = useCallback(() => {
    if (orderResetPending.current) return;
    orderResetPending.current = true;
    requestAnimationFrame(() => {
      orderResetPending.current = false;
      orderListRef.current?.resetAfterIndex(0);
    });
  }, []);

  // ── Stable expand/collapse handler (uses refs, never changes) ───────
  const handleToggleExpand = useCallback((openTime, force) => {
    const list = filteredPositionsRef.current;
    const idx = list.findIndex((p) => p.open_time === openTime);

    let changed = false;
    setExpandedPositions((prev) => {
      if (force === true) {
        if (prev.has(openTime)) return prev; // already expanded → bail out
        changed = true;
        const next = new Set(prev);
        next.add(openTime);
        return next;
      }
      if (force === false) {
        if (!prev.has(openTime)) return prev; // already collapsed → bail out
        changed = true;
        const next = new Set(prev);
        next.delete(openTime);
        return next;
      }
      // toggle
      changed = true;
      const next = new Set(prev);
      if (next.has(openTime)) next.delete(openTime);
      else next.add(openTime);
      return next;
    });

    // Only re-measure if the card actually changed size
    if (idx >= 0 && changed) {
      delete posSizeCache.current[idx];
      requestAnimationFrame(() => {
        posListRef.current?.resetAfterIndex(idx);
      });
    }
  }, []); // <-- STABLE: no deps, uses refs

  // Stable ref wrapper so row renderer can pass a stable callback
  const handleToggleExpandRef = useRef(handleToggleExpand);
  handleToggleExpandRef.current = handleToggleExpand;

  // ── List & container refs ────────────────────────────────────────────
  const posListRef = useRef(null);
  const orderListRef = useRef(null);
  const posObserverRef = useRef(null);
  const orderObserverRef = useRef(null);
  const posContainerElRef = useRef(null);
  const orderContainerElRef = useRef(null);

  // ── Container heights (via callback refs) ────────────────────────────
  const [posContainerHeight, setPosContainerHeight] = useState(0);
  const [orderContainerHeight, setOrderContainerHeight] = useState(0);

  const posContainerCbRef = useCallback((el) => {
    if (posObserverRef.current) {
      posObserverRef.current.disconnect();
      posObserverRef.current = null;
    }
    posContainerElRef.current = el;
    if (!el) return;
    // Fire immediately so we don't miss the initial size
    setPosContainerHeight(el.clientHeight || 0);
    const observer = new ResizeObserver((entries) => {
      const h = entries[0]?.contentRect?.height;
      if (h != null) setPosContainerHeight(h);
    });
    observer.observe(el);
    posObserverRef.current = observer;
  }, []);

  const orderContainerCbRef = useCallback((el) => {
    if (orderObserverRef.current) {
      orderObserverRef.current.disconnect();
      orderObserverRef.current = null;
    }
    orderContainerElRef.current = el;
    if (!el) return;
    setOrderContainerHeight(el.clientHeight || 0);
    const observer = new ResizeObserver((entries) => {
      const h = entries[0]?.contentRect?.height;
      if (h != null) setOrderContainerHeight(h);
    });
    observer.observe(el);
    orderObserverRef.current = observer;
  }, []);

  // ── getItemSize (cache-first, refs only — stable) ──────────────────
  const getPositionItemSize = useCallback((index) => {
    const cached = posSizeCache.current[index];
    return cached !== undefined ? cached + CARD_GAP : EST_POS;
  }, []);

  const getOrderItemSize = useCallback((index) => {
    const cached = orderSizeCache.current[index];
    return cached !== undefined ? cached + CARD_GAP : EST_ORD;
  }, []);

  // ── scrollToRecord (waits for DOM, not fixed frame counts) ──────────
  useImperativeHandle(ref, () => ({
    scrollToRecord(recordId, onReady) {
      const list = historyPositionList.filter(
        (v) => v.symbol === currentSymbolRef.current
      );
      let targetIndex = -1;
      let targetPos = null;
      for (let i = 0; i < list.length; i++) {
        if (list[i].log?.some((l) => l.id === recordId)) {
          targetIndex = i;
          targetPos = list[i];
          break;
        }
      }
      if (targetIndex === -1 || !targetPos) {
        if (onReady) onReady(null);
        return;
      }

      const capturedIndex = targetIndex;
      const capturedId = recordId;

      // Switch to positions tab
      onTabChange('positions');

      // Expand if not already expanded — only then do we need re-measurement
      const alreadyExpanded = expandedRef.current.has(targetPos.open_time);
      if (!alreadyExpanded) {
        setExpandedPositions((prev) => {
          if (prev.has(targetPos.open_time)) return prev;
          const next = new Set(prev);
          next.add(targetPos.open_time);
          return next;
        });
        delete posSizeCache.current[capturedIndex];
      }

      // Wait for React commit (tab switch + expand), then re-measure
      // and scroll, then poll for the DOM element to appear.
      requestAnimationFrame(() => {
        if (!alreadyExpanded) {
          posListRef.current?.resetAfterIndex(capturedIndex);
        }

        // Wait one frame for react-window to re-render the item
        requestAnimationFrame(() => {
          // Scroll the card into the virtual viewport
          posListRef.current?.scrollToItem(capturedIndex, 'start');

          // Poll for the record element to actually be in the DOM
          let attempts = 0;
          const waitForDom = () => {
            const el = document.getElementById('record_' + capturedId);
            if (el) {
              if (onReady) onReady(capturedId);
            } else if (attempts++ < 120) {
              requestAnimationFrame(waitForDom);
            } else {
              if (onReady) onReady(null); // timeout (~2 s)
            }
          };
          requestAnimationFrame(waitForDom);
        });
      });
    },
  }), [historyPositionList, onTabChange]);

  // ── STABLE row renderers (empty deps — never invalidated) ───────────

  // Measurement helper — uses rAF to avoid forced layout during render.
  const measureCard = useCallback((el, index, cacheRef, scheduleReset) => {
    if (!el) return;
    requestAnimationFrame(() => {
      if (!el.isConnected) return;
      const measured = Math.round(el.getBoundingClientRect().height);
      if (measured <= 0) return;
      if (cacheRef.current[index] !== measured) {
        cacheRef.current[index] = measured;
        scheduleReset();
      }
    });
  }, []);

  const renderPositionRow = useCallback(
    ({ index, style }) => {
      const list = filteredPositionsRef.current;
      const pos = list[index];
      if (!pos) return null;
      return (
        <div style={style}>
          <div
            ref={(el) => measureCard(el, index, posSizeCache, schedulePosReset)}
            style={{ padding: '0 14px' }}
          >
            <PositionCard
              position={pos}
              scrollToTime={scrollToTimeRef.current}
              isFirst={index === 0}
              expanded={expandedRef.current.has(pos.open_time)}
              onToggleExpand={(force) =>
                handleToggleExpandRef.current(pos.open_time, force)
              }
            />
          </div>
        </div>
      );
    },
    [measureCard, schedulePosReset]
  );

  const renderOrderRow = useCallback(
    ({ index, style }) => {
      const list = filteredOrdersRef.current;
      const order = list[index];
      if (!order) return null;
      return (
        <div style={style}>
          <div
            ref={(el) => measureCard(el, index, orderSizeCache, scheduleOrderReset)}
            style={{ padding: '0 14px' }}
          >
            <OrderCard
              order={order}
              scrollToTime={scrollToTimeRef.current}
            />
          </div>
        </div>
      );
    },
    [measureCard, scheduleOrderReset]
  );

  // ── Tab items ───────────────────────────────────────────────────────
  const items = useMemo(
    () => [
      {
        key: 'summary',
        label: isZh ? '总结' : 'Summary',
        children: <SummaryView />,
      },
      {
        key: 'positions',
        label: isZh ? '历史仓位' : 'History Position',
        children:
          filteredPositions.length === 0 ? (
            <Empty
              description={
                isZh ? '暂无历史仓位' : 'No history positions to display'
              }
              style={{ padding: 40 }}
            />
          ) : (
            <div ref={posContainerCbRef} style={{ height: '100%', width: '100%' }}>
              {posContainerHeight > 0 && (
                <VariableSizeList
                  ref={posListRef}
                  height={posContainerHeight}
                  width="100%"
                  itemCount={filteredPositions.length}
                  itemSize={getPositionItemSize}
                  estimatedItemSize={EST_POS}
                  overscanCount={OVERSCAN_COUNT}
                  style={{ overflowX: 'hidden' }}
                >
                  {renderPositionRow}
                </VariableSizeList>
              )}
            </div>
          ),
      },
      {
        key: 'orders',
        label: isZh ? '历史订单' : 'History Order',
        children:
          filteredOrders.length === 0 ? (
            <Empty
              description={
                isZh ? '暂无历史订单' : 'No history orders to display'
              }
              style={{ padding: 40 }}
            />
          ) : (
            <div ref={orderContainerCbRef} style={{ height: '100%', width: '100%' }}>
              {orderContainerHeight > 0 && (
                <VariableSizeList
                  ref={orderListRef}
                  height={orderContainerHeight}
                  width="100%"
                  itemCount={filteredOrders.length}
                  itemSize={getOrderItemSize}
                  estimatedItemSize={EST_ORD}
                  overscanCount={OVERSCAN_COUNT}
                  style={{ overflowX: 'hidden' }}
                >
                  {renderOrderRow}
                </VariableSizeList>
              )}
            </div>
          ),
      },
    ],
    [
      isZh,
      filteredPositions,
      filteredOrders,
      posContainerHeight,
      orderContainerHeight,
      posContainerCbRef,
      orderContainerCbRef,
      getPositionItemSize,
      getOrderItemSize,
      renderPositionRow,
      renderOrderRow,
    ]
  );

  return (
    <aside
      style={{
        flex: 1,
        minWidth: 320,
        maxWidth: 520,
        minHeight: 0,
        borderLeft: '1px solid var(--panel-border-color)',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      <Tabs
        className="history-tabs"
        items={items}
        activeKey={activeTab}
        onChange={onTabChange}
        size="small"
        tabBarStyle={{ padding: '0 14px', marginBottom: 0 }}
      />
    </aside>
  );
});

export default HistoryPanel;
