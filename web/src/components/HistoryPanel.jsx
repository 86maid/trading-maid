import React, { useState, useRef, useCallback, useEffect, useMemo, useImperativeHandle, forwardRef } from 'react';
import { VariableSizeList } from 'react-window';
import { Tabs, Empty } from 'antd';
import SummaryView from './SummaryView';
import PositionCard from './PositionCard';
import OrderCard from './OrderCard';
import { useTradingData } from '../context/TradingDataContext';

// ── Constants ──────────────────────────────────────────────────────────
const CARD_GAP = 12;
const OVERSCAN_COUNT = 3;
const TRADE_LOG_ROW_H = 31;   // padding(7+7) + line-height(~17) = 31
const TRADE_LOG_MAX_H = 200;  // maxHeight of the scrollable log area
const TRADE_LOG_CHROME = 17;  // marginTop(8) + borderTop(1) + paddingTop(8)
const EST_COLLAPSED = 250;    // fallback before first card is measured
const EST_ORDER = 210;

const HistoryPanel = forwardRef(function HistoryPanel(
  { scrollToTime, activeTab, onTabChange },
  ref
) {
  const { historyPositionList, historyOrderList, currentSymbol, isZh } =
    useTradingData();

  // ── Derived data ────────────────────────────────────────────────────
  const filteredPositions = useMemo(
    () => historyPositionList.filter((v) => v.symbol === currentSymbol),
    [historyPositionList, currentSymbol]
  );
  const filteredOrders = useMemo(
    () => historyOrderList.filter((v) => v.symbol === currentSymbol),
    [historyOrderList, currentSymbol]
  );

  // ── Refs (always fresh, never invalidate callbacks) ─────────────────
  const filteredPositionsRef = useRef(filteredPositions);
  filteredPositionsRef.current = filteredPositions;
  const filteredOrdersRef = useRef(filteredOrders);
  filteredOrdersRef.current = filteredOrders;
  const scrollToTimeRef = useRef(scrollToTime);
  scrollToTimeRef.current = scrollToTime;

  // ── Expanded state ──────────────────────────────────────────────────
  const [expandedPositions, setExpandedPositions] = useState(new Set());
  const expandedRef = useRef(expandedPositions);
  expandedRef.current = expandedPositions;

  // ── Measured collapsed heights (one value for all cards) ────────────
  const [posCollapsedH, setPosCollapsedH] = useState(null);
  const [orderCollapsedH, setOrderCollapsedH] = useState(null);

  // Measure the first card once, use that height for ALL collapsed cards.
  // This makes the total scroll height perfectly stable.
  const posFirstMeasureRef = useCallback((el) => {
    if (!el || posCollapsedH !== null) return;
    requestAnimationFrame(() => {
      if (!el.isConnected) return;
      const h = Math.round(el.getBoundingClientRect().height);
      if (h > 0) setPosCollapsedH(h);
    });
  }, [posCollapsedH]);

  const orderFirstMeasureRef = useCallback((el) => {
    if (!el || orderCollapsedH !== null) return;
    requestAnimationFrame(() => {
      if (!el.isConnected) return;
      const h = Math.round(el.getBoundingClientRect().height);
      if (h > 0) setOrderCollapsedH(h);
    });
  }, [orderCollapsedH]);

  // When the measured height arrives, re-layout once.
  useEffect(() => {
    if (posCollapsedH !== null) {
      requestAnimationFrame(() => posListRef.current?.resetAfterIndex(0));
    }
  }, [posCollapsedH]);

  useEffect(() => {
    if (orderCollapsedH !== null) {
      requestAnimationFrame(() => orderListRef.current?.resetAfterIndex(0));
    }
  }, [orderCollapsedH]);

  // ── List refs ────────────────────────────────────────────────────────
  const posListRef = useRef(null);
  const orderListRef = useRef(null);

  // ── Expand / collapse handler ────────────────────────────────────────
  const handleToggleExpand = useCallback((openTime, force) => {
    const list = filteredPositionsRef.current;
    const idx = list.findIndex((p) => p.open_time === openTime);

    let changed = false;
    setExpandedPositions((prev) => {
      if (force === true) {
        if (prev.has(openTime)) return prev;
        changed = true;
        const next = new Set(prev); next.add(openTime); return next;
      }
      if (force === false) {
        if (!prev.has(openTime)) return prev;
        changed = true;
        const next = new Set(prev); next.delete(openTime); return next;
      }
      changed = true;
      const next = new Set(prev);
      if (next.has(openTime)) next.delete(openTime);
      else next.add(openTime);
      return next;
    });

    if (idx >= 0 && changed) {
      requestAnimationFrame(() => posListRef.current?.resetAfterIndex(idx));
    }
  }, []);

  const handleToggleExpandRef = useRef(handleToggleExpand);
  handleToggleExpandRef.current = handleToggleExpand;

  // ── Container height measurement ─────────────────────────────────────
  const [posContainerHeight, setPosContainerHeight] = useState(0);
  const [orderContainerHeight, setOrderContainerHeight] = useState(0);
  const posObserverRef = useRef(null);
  const orderObserverRef = useRef(null);

  const posContainerCbRef = useCallback((el) => {
    if (posObserverRef.current) { posObserverRef.current.disconnect(); posObserverRef.current = null; }
    if (!el) return;
    setPosContainerHeight(el.clientHeight || 0);
    const obs = new ResizeObserver((e) => {
      const h = e[0]?.contentRect?.height;
      if (h != null) setPosContainerHeight(h);
    });
    obs.observe(el);
    posObserverRef.current = obs;
  }, []);

  const orderContainerCbRef = useCallback((el) => {
    if (orderObserverRef.current) { orderObserverRef.current.disconnect(); orderObserverRef.current = null; }
    if (!el) return;
    setOrderContainerHeight(el.clientHeight || 0);
    const obs = new ResizeObserver((e) => {
      const h = e[0]?.contentRect?.height;
      if (h != null) setOrderContainerHeight(h);
    });
    obs.observe(el);
    orderObserverRef.current = obs;
  }, []);

  // ── getItemSize — deterministic, no per-item measurement cache ──────
  const getPositionItemSize = useCallback((index) => {
    const base = posCollapsedH ?? EST_COLLAPSED;
    const pos = filteredPositionsRef.current[index];
    if (pos && expandedRef.current.has(pos.open_time)) {
      const rows = pos.log?.length || 0;
      const logH = Math.min(rows * TRADE_LOG_ROW_H, TRADE_LOG_MAX_H);
      return base + TRADE_LOG_CHROME + logH + CARD_GAP;
    }
    return base + CARD_GAP;
  }, [posCollapsedH]);

  const getOrderItemSize = useCallback(() => {
    return (orderCollapsedH ?? EST_ORDER) + CARD_GAP;
  }, [orderCollapsedH]);

  // ── scrollToRecord ──────────────────────────────────────────────────
  useImperativeHandle(ref, () => ({
    scrollToRecord(recordId, onReady) {
      const list = historyPositionList.filter(v => v.symbol === currentSymbol);
      let idx = -1, target = null;
      for (let i = 0; i < list.length; i++) {
        if (list[i].log?.some(l => l.id === recordId)) { idx = i; target = list[i]; break; }
      }
      if (idx === -1 || !target) { onReady?.(null); return; }

      const capIdx = idx, capId = recordId;
      onTabChange('positions');

      if (!expandedRef.current.has(target.open_time)) {
        setExpandedPositions(prev => {
          if (prev.has(target.open_time)) return prev;
          const n = new Set(prev); n.add(target.open_time); return n;
        });
        requestAnimationFrame(() => {
          posListRef.current?.resetAfterIndex(capIdx);
          requestAnimationFrame(() => {
            posListRef.current?.scrollToItem(capIdx, 'start');
            requestAnimationFrame(() => pollDom(capId, onReady));
          });
        });
      } else {
        requestAnimationFrame(() => {
          posListRef.current?.scrollToItem(capIdx, 'start');
          requestAnimationFrame(() => pollDom(capId, onReady));
        });
      }

      function pollDom(id, cb) {
        let n = 0;
        const check = () => {
          if (document.getElementById('record_' + id)) { cb?.(id); }
          else if (n++ < 120) requestAnimationFrame(check);
          else cb?.(null);
        };
        requestAnimationFrame(check);
      }
    },
  }), [historyPositionList, onTabChange]);

  // ── Row renderers (stable deps) ──────────────────────────────────────
  const renderPositionRow = useCallback(
    ({ index, style }) => {
      const pos = filteredPositionsRef.current[index];
      if (!pos) return null;
      return (
        <div style={style}>
          <div
            ref={index === 0 ? posFirstMeasureRef : undefined}
            style={{ padding: '0 14px' }}
          >
            <PositionCard
              position={pos}
              scrollToTime={scrollToTimeRef.current}
              expanded={expandedRef.current.has(pos.open_time)}
              onToggleExpand={(f) => handleToggleExpandRef.current(pos.open_time, f)}
            />
          </div>
        </div>
      );
    },
    [posFirstMeasureRef]
  );

  const renderOrderRow = useCallback(
    ({ index, style }) => {
      const order = filteredOrdersRef.current[index];
      if (!order) return null;
      return (
        <div style={style}>
          <div
            ref={index === 0 ? orderFirstMeasureRef : undefined}
            style={{ padding: '0 14px' }}
          >
            <OrderCard order={order} scrollToTime={scrollToTimeRef.current} />
          </div>
        </div>
      );
    },
    [orderFirstMeasureRef]
  );

  // ── Tab items ───────────────────────────────────────────────────────
  const items = useMemo(() => [
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
          <Empty description={isZh ? '暂无历史仓位' : 'No history positions'} style={{ padding: 40 }} />
        ) : (
          <div ref={posContainerCbRef} style={{ height: '100%', width: '100%' }}>
            {posContainerHeight > 0 && (
              <VariableSizeList
                ref={posListRef}
                height={posContainerHeight}
                width="100%"
                itemCount={filteredPositions.length}
                itemSize={getPositionItemSize}
                estimatedItemSize={(posCollapsedH ?? EST_COLLAPSED) + CARD_GAP}
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
          <Empty description={isZh ? '暂无历史订单' : 'No history orders'} style={{ padding: 40 }} />
        ) : (
          <div ref={orderContainerCbRef} style={{ height: '100%', width: '100%' }}>
            {orderContainerHeight > 0 && (
              <VariableSizeList
                ref={orderListRef}
                height={orderContainerHeight}
                width="100%"
                itemCount={filteredOrders.length}
                itemSize={getOrderItemSize}
                estimatedItemSize={(orderCollapsedH ?? EST_ORDER) + CARD_GAP}
                overscanCount={OVERSCAN_COUNT}
                style={{ overflowX: 'hidden' }}
              >
                {renderOrderRow}
              </VariableSizeList>
            )}
          </div>
        ),
    },
  ], [
    isZh, filteredPositions, filteredOrders,
    posContainerHeight, orderContainerHeight,
    posContainerCbRef, orderContainerCbRef,
    getPositionItemSize, getOrderItemSize,
    renderPositionRow, renderOrderRow,
    posCollapsedH, orderCollapsedH,
  ]);

  return (
    <aside style={{
      flex: 1, minWidth: 320, maxWidth: 520, minHeight: 0,
      borderLeft: '1px solid var(--panel-border-color)',
      display: 'flex', flexDirection: 'column', overflow: 'hidden',
    }}>
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
