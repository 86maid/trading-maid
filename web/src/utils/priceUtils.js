export function makePriceFormatter(tickSize) {
  return (v) => {
    const snapped = Math.round(Number(v) / tickSize) * tickSize;
    const precision = (tickSize.toString().split('.')[1] || '').length;
    return snapped.toFixed(precision);
  };
}

export function makeQtyFormatter(minSize) {
  return (value) => {
    const n = Number(value);
    if (!Number.isFinite(n)) return '-';
    const snapped = Math.round(n / minSize) * minSize;
    const precision = (minSize.toString().split('.')[1] || '').length;
    return snapped.toFixed(precision);
  };
}

export function enumText(value) {
  if (value === undefined || value === null || value === '') {
    return '-';
  }
  return String(value);
}

export function priceText(value, priceFormatter) {
  const n = Number(value);
  if (!Number.isFinite(n)) return '-';
  return priceFormatter(n);
}

export function qtyText(value, qtyFormatter) {
  return qtyFormatter(value);
}

export function kindText(value, isZh) {
  const key = enumText(value);
  const map = {
    Trigger: isZh ? '触发单' : 'Trigger',
    Market: isZh ? '市价单' : 'Market',
    Limit: isZh ? '限价单' : 'Limit',
    Liquidation: isZh ? '强平单' : 'Liquidation',
    ADL: isZh ? '自动减仓' : 'ADL',
  };
  return map[key] || key;
}

export function statusText(value, kind, isZh) {
  const key = enumText(value);
  const map = {
    Submitted: isZh ? '已提交' : 'Submitted',
    PartiallyFilled: isZh ? '部分成交' : 'Partially Filled',
    Filled: kind === 'Trigger' ? (isZh ? '已触发' : 'Triggered') : (isZh ? '已成交' : 'Filled'),
    Canceled: isZh ? '已取消' : 'Canceled',
    Rejected: isZh ? '已拒绝' : 'Rejected',
  };
  return map[key] || key;
}
