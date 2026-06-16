import React, { useEffect, useRef } from 'react';
import { Flex, Typography } from 'antd';

const { Text } = Typography;

export default function TradeLogRow({ log, priceFormatter, isZh, isFlashing, onClick }) {
  const rowRef = useRef(null);

  const sideText = log.side === 'Buy' ? (isZh ? '买' : 'Buy') : isZh ? '卖' : 'Sell';
  const kindText =
    log.kind === 'Liquidation'
      ? isZh
        ? '强平'
        : 'Liquidation'
      : sideText;

  const isBuy = log.side === 'Buy';
  const sideColor = isBuy ? 'var(--buy-color)' : 'var(--sell-color)';

  useEffect(() => {
    if (!rowRef.current) return;
    if (isFlashing) {
      const style = getComputedStyle(document.body);
      const color = style.getPropertyValue('--highlight-color').trim();
      rowRef.current.style.backgroundColor = color;
    } else {
      rowRef.current.style.backgroundColor = '';
    }
    return () => {
      if (rowRef.current) rowRef.current.style.backgroundColor = '';
    };
  }, [isFlashing]);

  return (
    <Flex
      ref={rowRef}
      gap="small"
      data-log-row
      id={`record_${log.id}`}
      onClick={onClick}
      style={{
        padding: '7px 6px',
        cursor: 'pointer',
        fontSize: 11,
        borderRadius: 6,
      }}
    >
      <Text style={{ flex: 2, fontSize: 11 }}>
        {new Date(log.time).toLocaleString()}
      </Text>
      <Text style={{ flex: 1, fontSize: 11, fontFamily: 'var(--font-mono)' }}>
        {priceFormatter(log.price)}
      </Text>
      <Text style={{ flex: 1, fontSize: 11, fontFamily: 'var(--font-mono)' }}>
        {log.quantity}
      </Text>
      <Text strong style={{ flex: 1, textAlign: 'right', fontSize: 11, color: sideColor }}>
        {kindText}
      </Text>
    </Flex>
  );
}
