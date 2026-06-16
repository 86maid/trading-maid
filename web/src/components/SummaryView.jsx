import React, { useMemo } from 'react';
import { Card, Empty } from 'antd';
import { useTradingData } from '../context/TradingDataContext';
import { computeSummary } from '../utils/summaryUtils';

export default function SummaryView() {
  const { historyPositionList, currentSymbol, isZh } = useTradingData();

  const stats = useMemo(
    () => computeSummary(historyPositionList, currentSymbol),
    [historyPositionList, currentSymbol]
  );

  if (stats.totalTrades === 0) {
    return (
      <Empty
        description={
          isZh ? '暂无可统计的历史仓位' : 'No historical positions to summarize'
        }
        style={{ padding: 40 }}
      />
    );
  }

  return (
    <div className="summary-grid">
      {stats.rows.map((row, i) => (
        <Card key={i} className="summary-card" size="small">
          <div className="summary-card-label">{row.label}</div>
          <div className={`summary-card-value ${row.cls}`}>{row.value}</div>
        </Card>
      ))}
    </div>
  );
}
