import React, { useEffect, useRef } from 'react';

export default function TradeLogRow({ log, priceFormatter, isZh, isFlashing, onClick }) {
  const rowRef = useRef(null);

  const sideText = log.side === 'Buy' ? (isZh ? '买' : 'Buy') : isZh ? '卖' : 'Sell';
  const kindText =
    log.kind === 'Liquidation'
      ? isZh
        ? '强平'
        : 'Liquidation'
      : sideText;

  useEffect(() => {
    if (!rowRef.current) return;
    if (isFlashing) {
      const style = getComputedStyle(document.body);
      const color = style.getPropertyValue('--highlight-color').trim();
      rowRef.current.style.backgroundColor = color;
    } else {
      rowRef.current.style.backgroundColor = '';
    }
    // cleanup on unmount or before next effect run
    return () => {
      if (rowRef.current) rowRef.current.style.backgroundColor = '';
    };
  }, [isFlashing]);

  const isBuy = log.side === 'Buy';

  return (
    <div
      ref={rowRef}
      className={`position-card-section ${isBuy ? 'position-card-side-buy' : 'position-card-side-sell'}`}
      id={`record_${log.id}`}
      onClick={onClick}
    >
      <div>{new Date(log.time).toLocaleString()}</div>
      <div>{priceFormatter(log.price)}</div>
      <div>{log.quantity}</div>
      <div>{kindText}</div>
    </div>
  );
}
