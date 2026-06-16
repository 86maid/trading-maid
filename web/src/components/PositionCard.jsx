import React, { useState, useEffect, useCallback, useRef } from 'react';
import { Card, Tag, Flex } from 'antd';
import TradeLogRow from './TradeLogRow';
import { useTradingData } from '../context/TradingDataContext';
import { makePriceFormatter } from '../utils/priceUtils';
import { t } from '../utils/i18n';

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

  // Expand first card by default
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
    if (e.target.closest('.position-card-section')) return;
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
  const closeReturnPctClass =
    closeReturnPct == null
      ? ''
      : closeReturnPct >= 0
        ? 'position-card-value-profit-positive'
        : 'position-card-value-profit-negative';

  const profitClass = (v) =>
    Number(v) >= 0
      ? 'position-card-value-profit-positive'
      : 'position-card-value-profit-negative';
  const profitSign = (v) => (Number(v) >= 0 ? '+' : '');

  const tagColor = isBuy ? 'red' : 'green';

  return (
    <Card
      className="order-card position-card"
      size="small"
      onClick={handleCardClick}
      hoverable
      data-open-time={position.open_time}
    >
      {/* Head */}
      <div className="order-card-head">
        <div className="order-card-title">{position.symbol}</div>
        <Flex gap="2px" wrap="wrap">
          <Tag color={tagColor}>{statusText}</Tag>
          <Tag color={tagColor}>{position.leverage}x</Tag>
          <Tag color={tagColor}>{t('Isolated', '逐仓', isZh)}</Tag>
          <Tag color={tagColor}>
            {isBuy ? t('Buy', '买', isZh) : t('Sell', '卖', isZh)}
          </Tag>
        </Flex>
      </div>

      {/* Stats grid */}
      <div className="order-card-grid position-card-grid">
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Entry Price', '开仓均价', isZh)}
          </div>
          <div className="order-card-value">
            {formatPrice(position.open_avg_price)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Max Position Size', '最大持仓量', isZh)}
          </div>
          <div className="order-card-value">{position.max_quantity}</div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Exit Price', '平仓均价', isZh)}
          </div>
          <div className="order-card-value">
            {formatPrice(position.close_avg_price)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Close Quantity', '平仓量', isZh)}
          </div>
          <div className="order-card-value">{position.close_quantity}</div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Net PnL', '净盈亏', isZh)}
          </div>
          <div className={`order-card-value ${profitClass(position.total_profit)}`}>
            {profitSign(position.total_profit)}
            {position.total_profit}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Rate of Return', '收益率', isZh)}%
          </div>
          <div className={`order-card-value ${closeReturnPctClass}`}>
            {closeReturnPctText}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Gross PnL', '毛盈亏', isZh)}
          </div>
          <div className={`order-card-value ${profitClass(position.profit)}`}>
            {profitSign(position.profit)}
            {position.profit}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">{t('Fee', '手续费', isZh)}</div>
          <div className="order-card-value position-card-value-profit-negative">
            -{position.fee}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Entry Time', '开仓时间', isZh)}
          </div>
          <div className="order-card-value">
            {new Date(position.open_time).toLocaleString()}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Exit Time', '平仓时间', isZh)}
          </div>
          <div className="order-card-value">
            {new Date(position.close_time).toLocaleString()}
          </div>
        </div>
      </div>

      {/* Expandable trade log */}
      {logExpanded && (
        <div className="log" style={{ display: 'block' }}>
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
