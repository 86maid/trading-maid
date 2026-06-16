import React from 'react';
import { Card, Tag, Flex } from 'antd';
import { useTradingData } from '../context/TradingDataContext';
import {
  makePriceFormatter,
  makeQtyFormatter,
  enumText,
  priceText,
  qtyText,
  kindText,
  statusText,
} from '../utils/priceUtils';
import { t } from '../utils/i18n';

export default function OrderCard({ order, scrollToTime }) {
  const { isZh, currentDataSource } = useTradingData();

  const priceFormatter = currentDataSource
    ? makePriceFormatter(currentDataSource.metadata.tick_size)
    : (v) => String(v);
  const qtyFormatter = currentDataSource
    ? makeQtyFormatter(currentDataSource.metadata.min_size)
    : (v) => String(v);

  const side = enumText(order.side);
  const isBuy = side === 'Buy';
  const createTime = Number(order.create_time);
  const updateTime = Number(order.update_time);
  const tagColor = isBuy ? 'red' : 'green';

  return (
    <Card
      className="order-card"
      size="small"
      hoverable
      onClick={() => scrollToTime(order.update_time)}
    >
      <div className="order-card-head">
        <div className="order-card-title">{enumText(order.id)}</div>
        <Flex gap="2px" wrap="wrap">
          {order.reduce_only && (
            <Tag color={tagColor}>
              {t('Reduce Only', '只减仓', isZh)}
            </Tag>
          )}
          <Tag color={tagColor}>{statusText(order.status, order.kind, isZh)}</Tag>
          <Tag color={tagColor}>{kindText(order.kind, isZh)}</Tag>
          <Tag color={tagColor}>
            {side === 'Buy'
              ? t('Buy', '买', isZh)
              : side === 'Sell'
                ? t('Sell', '卖', isZh)
                : side}
          </Tag>
        </Flex>
      </div>
      <div className="order-card-grid">
        <div className="order-card-item">
          <div className="order-card-label">{t('Symbol', '交易对', isZh)}</div>
          <div className="order-card-value">{enumText(order.symbol)}</div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Trigger Price', '触发价', isZh)}
          </div>
          <div className="order-card-value">
            {priceText(order.trigger_price, priceFormatter)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Order Price', '委托价', isZh)}
          </div>
          <div className="order-card-value">
            {priceText(order.price, priceFormatter)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Quantity', '数量', isZh)}
          </div>
          <div className="order-card-value">
            {qtyText(order.quantity, qtyFormatter)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Average Fill', '成交均价', isZh)}
          </div>
          <div className="order-card-value">
            {priceText(order.avg_price, priceFormatter)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Cumulative Qty', '累计成交', isZh)}
          </div>
          <div className="order-card-value">
            {qtyText(order.cumulative_quantity, qtyFormatter)}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Create Time', '创建时间', isZh)}
          </div>
          <div className="order-card-value">
            {Number.isFinite(createTime)
              ? new Date(createTime).toLocaleString()
              : '-'}
          </div>
        </div>
        <div className="order-card-item">
          <div className="order-card-label">
            {t('Update Time', '更新时间', isZh)}
          </div>
          <div className="order-card-value">
            {Number.isFinite(updateTime)
              ? new Date(updateTime).toLocaleString()
              : '-'}
          </div>
        </div>
      </div>
    </Card>
  );
}
