import React, { useState, useEffect, useCallback, useRef } from 'react';
import { Card, Tag, Flex, Typography, Descriptions } from 'antd';
import TradeLogRow from './TradeLogRow';
import { useTradingData } from '../context/TradingDataContext';
import { makePriceFormatter } from '../utils/priceUtils';
import { t } from '../utils/i18n';

const { Text } = Typography;

export default function PositionCard({ position, scrollToTime, isFirst }) {
  const { isZh, currentDataSource } = useTradingData();
  const [logExpanded, setLogExpanded] = useState(false);
  const [flashId, setFlashId] = useState(null);
  const flashTimerRef = useRef(null);

  const isBuy = position.side === 'Buy';
  const isLiquidation =
    position.log.length > 0 &&
    position.log[position.log.length - 1].kind === 'Liquidation';
  const isFullClose = position.max_quantity === position.close_quantity;

  const statusText = isLiquidation
    ? t('Liquidation', '强平', isZh)
    : isFullClose
      ? t('Full Close', '完全平仓', isZh)
      : t('Partial Close', '部分平仓', isZh);

  useEffect(() => {
    if (isFirst) setLogExpanded(true);
  }, [isFirst]);

  const priceFormatter = currentDataSource
    ? makePriceFormatter(currentDataSource.metadata.tick_size)
    : (v) => String(v);

  const formatPrice = (v) => {
    const n = Number(v);
    if (!Number.isFinite(n)) return '-';
    return priceFormatter(n);
  };

  const handleCardClick = (e) => {
    if (e.target.closest('[data-log-row]')) return;
    setLogExpanded((prev) => !prev);
  };

  const handleLogClick = useCallback(
    (log, index) => {
      setLogExpanded(true);
      scrollToTime(log.time);
      setFlashId(log.id);
      if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
      flashTimerRef.current = setTimeout(() => setFlashId(null), 1000);
    },
    [scrollToTime]
  );

  const tagColor = isBuy ? 'red' : 'green';

  // Profit color helpers — use CSS variables for consistency with chart theme
  const profitColor = (v) =>
    Number(v) >= 0
      ? 'var(--profit-positive-color)'
      : 'var(--profit-negative-color)';
  const profitSign = (v) => (Number(v) >= 0 ? '+' : '');

  // Computed values
  const openAvgPrice = Number(position.open_avg_price);
  const maxQuantity = Number(position.max_quantity);
  const leverage = Number(position.leverage);
  const profit = Number(position.profit);
  const initialMargin =
    Number.isFinite(openAvgPrice) &&
      Number.isFinite(maxQuantity) &&
      Number.isFinite(leverage) &&
      leverage > 0
      ? (openAvgPrice * maxQuantity) / leverage
      : null;
  const closeReturnPct =
    Number.isFinite(profit) &&
      Number.isFinite(initialMargin) &&
      initialMargin > 0
      ? (profit / initialMargin) * 100
      : null;
  const closeReturnPctText =
    closeReturnPct == null
      ? '-'
      : `${closeReturnPct >= 0 ? '+' : ''}${closeReturnPct.toFixed(2)}%`;

  return (
    <Card
      size="small"
      onClick={handleCardClick}
      hoverable
      data-open-time={position.open_time}
      style={{ marginBottom: 0 }}
    >
      {/* Head */}
      <Flex justify="space-between" align="flex-start" gap={8} style={{ marginBottom: 8 }}>
        <Text strong style={{ fontFamily: 'var(--font-mono)', fontSize: 14, flexShrink: 0, marginTop: 2 }}>
          {position.symbol}
        </Text>
        <Flex gap="2px 4px" wrap="wrap" style={{ justifyContent: 'flex-end' }}>
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>{statusText}</Tag>
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>{position.leverage}x</Tag>
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>{t('Isolated', '逐仓', isZh)}</Tag>
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>
            {isBuy ? t('Buy', '买', isZh) : t('Sell', '卖', isZh)}
          </Tag>
        </Flex>
      </Flex>

      {/* Stats grid */}
      <Descriptions size="small" column={2} colon={false}>
        <Descriptions.Item label={t('Entry Price', '开仓均价', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {formatPrice(position.open_avg_price)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Max Position Size', '最大持仓量', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {position.max_quantity}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Exit Price', '平仓均价', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {formatPrice(position.close_avg_price)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Close Quantity', '平仓量', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {position.close_quantity}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Net PnL', '净盈亏', isZh)}>
          <Text
            strong
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              color: profitColor(position.total_profit),
            }}
          >
            {profitSign(position.total_profit)}
            {position.total_profit}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={`${t('Rate of Return', '收益率', isZh)}%`}>
          <Text
            strong
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              color:
                closeReturnPct == null
                  ? undefined
                  : closeReturnPct >= 0
                    ? 'var(--profit-positive-color)'
                    : 'var(--profit-negative-color)',
            }}
          >
            {closeReturnPctText}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Gross PnL', '毛盈亏', isZh)}>
          <Text
            strong
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              color: profitColor(position.profit),
            }}
          >
            {profitSign(position.profit)}
            {position.profit}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Fee', '手续费', isZh)}>
          <Text
            strong
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              color: 'var(--profit-negative-color)',
            }}
          >
            -{position.fee}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Entry Time', '开仓时间', isZh)}>
          <Text style={{ fontSize: 12 }}>
            {new Date(position.open_time).toLocaleString()}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Exit Time', '平仓时间', isZh)}>
          <Text style={{ fontSize: 12 }}>
            {new Date(position.close_time).toLocaleString()}
          </Text>
        </Descriptions.Item>
      </Descriptions>

      {/* Expandable trade log */}
      {logExpanded && (
        <div
          data-section="trade-log"
          style={{
            display: 'block',
            marginTop: 8,
            borderTop: '1px solid var(--panel-border-color)',
            paddingTop: 8,
          }}
        >
          {position.log.map((log, index) => (
            <TradeLogRow
              key={log.id}
              log={log}
              priceFormatter={formatPrice}
              isZh={isZh}
              isFlashing={flashId === log.id}
              onClick={() => handleLogClick(log, index)}
            />
          ))}
        </div>
      )}
    </Card>
  );
}
