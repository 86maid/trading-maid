export const KIND_TEXT = {
  Trigger: { en: 'Trigger', zh: '触发单' },
  Marker: { en: 'Market', zh: '市价单' },
  Limit: { en: 'Limit', zh: '限价单' },
  Liquidation: { en: 'Liquidation', zh: '强平单' },
  ADL: { en: 'ADL', zh: '自动减仓' },
};

export const STATUS_TEXT = {
  Submitted: { en: 'Submitted', zh: '已提交' },
  PartiallyFilled: { en: 'Partially Filled', zh: '部分成交' },
  Filled: { en: 'Filled', zh: '已成交' },
  Canceled: { en: 'Canceled', zh: '已取消' },
  Rejected: { en: 'Rejected', zh: '已拒绝' },
};

export const LOCALE_DATE_FORMATS = {
  'zh-CN': 'yyyy-MM-dd HH:mm',
  'en-US': 'MMM dd, yyyy hh:mm A',
  'en-GB': 'dd MMM yyyy HH:mm',
  'en-CA': 'yyyy-MM-dd HH:mm',
  'en-AU': 'dd/MM/yyyy HH:mm',
  'fr-FR': 'dd/MM/yyyy HH:mm',
  'de-DE': 'dd.MM.yyyy HH:mm',
  'ja-JP': 'yyyy-MM-dd HH:mm',
  'ko-KR': 'yyyy.MM.dd HH:mm',
  'ru-RU': 'dd.MM.yyyy HH:mm',
  'es-ES': 'dd/MM/yyyy HH:mm',
  'it-IT': 'dd/MM/yyyy HH:mm',
};

export const TickMarkType = {
  Year: 0,
  Month: 1,
  DayOfMonth: 2,
  Time: 3,
  TimeWithSeconds: 4,
};
