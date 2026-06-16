import React from 'react';
import { Tabs, Empty } from 'antd';
import SummaryView from './SummaryView';
import PositionCard from './PositionCard';
import OrderCard from './OrderCard';
import { useTradingData } from '../context/TradingDataContext';

export default function HistoryPanel({ scrollToTime, activeTab, onTabChange }) {
  const { historyPositionList, historyOrderList, currentSymbol, isZh } =
    useTradingData();

  const filteredPositions = historyPositionList.filter(
    (v) => v.symbol === currentSymbol
  );
  const filteredOrders = historyOrderList.filter(
    (v) => v.symbol === currentSymbol
  );

  const scrollContainerStyle = {
    overflow: 'auto',
    padding: 14,
    display: 'flex',
    flexDirection: 'column',
    gap: 12,
    flex: 1,
    minHeight: 0,
  };

  const items = [
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
          <div id="history-position-container" style={scrollContainerStyle}>
            {filteredPositions.map((position, i) => (
              <PositionCard
                key={`${position.symbol}-${position.open_time}-${i}`}
                position={position}
                scrollToTime={scrollToTime}
                isFirst={i === 0}
              />
            ))}
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
          <div id="history-order-container" style={scrollContainerStyle}>
            {filteredOrders.map((order, i) => (
              <OrderCard
                key={order.id || i}
                order={order}
                scrollToTime={scrollToTime}
              />
            ))}
          </div>
        ),
    },
  ];

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
      <div
        style={{
          flex: 1,
          minHeight: 0,
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        <Tabs
          items={items}
          activeKey={activeTab}
          onChange={onTabChange}
          size="small"
          tabBarStyle={{ padding: '0 14px', marginBottom: 0 }}
          style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}
        />
      </div>
    </aside>
  );
}
