import React, { useMemo } from 'react';
import { Card, Empty, Row, Col, Typography } from 'antd';
import { useTradingData } from '../context/TradingDataContext';
import { computeSummary } from '../utils/summaryUtils';

const { Text } = Typography;

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
    <Row gutter={[10, 10]} style={{ padding: 14 }}>
      {stats.rows.map((row, i) => (
        <Col key={i} span={12}>
          <Card size="small">
            <Text
              type="secondary"
              style={{
                fontSize: 11,
                textTransform: 'uppercase',
                letterSpacing: '0.08em',
              }}
            >
              {row.label}
            </Text>
            <br />
            <Text
              strong
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 18,
                color:
                  row.cls === 'positive'
                    ? 'var(--profit-positive-color)'
                    : row.cls === 'negative'
                      ? 'var(--profit-negative-color)'
                      : undefined,
              }}
            >
              {row.value}
            </Text>
          </Card>
        </Col>
      ))}
    </Row>
  );
}
