import { getTimeRange } from './timeUtils';

export function createMaker(position, locale, level, buyColor, sellColor) {
  const isZh = (locale || '').startsWith('zh');
  const buy = isZh ? '买' : 'Buy';
  const sell = isZh ? '卖' : 'Sell';

  return position.log.map((v) => {
    if (v.side === 'Buy') {
      return {
        id: v.id,
        time: getTimeRange(v.time, level)[0],
        position: 'belowBar',
        color: buyColor,
        shape: 'arrowUp',
        text: buy,
      };
    } else {
      return {
        id: v.id,
        time: getTimeRange(v.time, level)[0],
        position: 'aboveBar',
        color: sellColor,
        shape: 'arrowDown',
        text: sell,
      };
    }
  });
}
