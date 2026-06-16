import React from 'react';
import { Card, Tag, Flex, Typography, Descriptions } from 'antd';
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

const { Text } = Typography;

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
      size="small"
      hoverable
      onClick={() => scrollToTime(order.update_time)}
      style={{ marginBottom: 0 }}
    >
      <Flex justify="space-between" align="flex-start" gap={8} style={{ marginBottom: 8 }}>
        <Text strong style={{ fontFamily: 'var(--font-mono)', fontSize: 14, flexShrink: 0, marginTop: 2 }}>
          {enumText(order.id)}
        </Text>
        <Flex gap="2px 4px" wrap="wrap" style={{ justifyContent: 'flex-end' }}>
          {order.reduce_only && (
            <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>
              {t('Reduce Only', '只减仓', isZh)}
            </Tag>
          )}
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>{statusText(order.status, order.kind, isZh)}</Tag>
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>{kindText(order.kind, isZh)}</Tag>
          <Tag color={tagColor} style={{ fontSize: 11, margin: 0 }}>
            {side === 'Buy'
              ? t('Buy', '买', isZh)
              : side === 'Sell'
                ? t('Sell', '卖', isZh)
                : side}
          </Tag>
        </Flex>
      </Flex>

      <Descriptions size="small" column={2} colon={false}>
        <Descriptions.Item label={t('Symbol', '交易对', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {enumText(order.symbol)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Trigger Price', '触发价', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {priceText(order.trigger_price, priceFormatter)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Order Price', '委托价', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {priceText(order.price, priceFormatter)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Quantity', '数量', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {qtyText(order.quantity, qtyFormatter)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Average Fill', '成交均价', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {priceText(order.avg_price, priceFormatter)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Cumulative Qty', '累计成交', isZh)}>
          <Text style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
            {qtyText(order.cumulative_quantity, qtyFormatter)}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Create Time', '创建时间', isZh)}>
          <Text style={{ fontSize: 12 }}>
            {Number.isFinite(createTime)
              ? new Date(createTime).toLocaleString()
              : '-'}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label={t('Update Time', '更新时间', isZh)}>
          <Text style={{ fontSize: 12 }}>
            {Number.isFinite(updateTime)
              ? new Date(updateTime).toLocaleString()
              : '-'}
          </Text>
        </Descriptions.Item>
      </Descriptions>
    </Card>
  );
}
