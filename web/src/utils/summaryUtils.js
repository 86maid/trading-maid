export function computeSummary(historyPositionList, symbol) {
  const list = symbol
    ? historyPositionList.filter((v) => v.symbol === symbol)
    : historyPositionList;

  if (list.length === 0) {
    return { totalTrades: 0, rows: [] };
  }

  const totalTrades = list.length;
  const totalProfit = list.reduce((acc, v) => acc + Number(v.total_profit || 0), 0);
  const totalFee = list.reduce((acc, v) => acc + Number(v.fee || 0), 0);
  const winTrades = list.filter((v) => Number(v.total_profit || 0) > 0).length;
  const lossTrades = list.filter((v) => Number(v.total_profit || 0) < 0).length;
  const winRate = totalTrades === 0 ? 0 : (winTrades / totalTrades) * 100;
  const avgProfit = totalTrades === 0 ? 0 : totalProfit / totalTrades;
  const netProfits = list.map((v) => Number(v.total_profit || 0));
  const winProfits = netProfits.filter((v) => v > 0);
  const lossProfits = netProfits.filter((v) => v < 0);
  const avgWin =
    winProfits.length === 0
      ? 0
      : winProfits.reduce((acc, v) => acc + v, 0) / winProfits.length;
  const avgLoss =
    lossProfits.length === 0
      ? 0
      : Math.abs(lossProfits.reduce((acc, v) => acc + v, 0)) / lossProfits.length;
  const profitLossRatio = avgLoss === 0 ? null : avgWin / avgLoss;
  const netGrossProfit = winProfits.reduce((acc, v) => acc + v, 0);
  const netGrossLossAbs = Math.abs(lossProfits.reduce((acc, v) => acc + v, 0));
  const grossPnLList = list.map((v) => Number(v.profit || 0));
  const grossProfit = grossPnLList
    .filter((v) => v > 0)
    .reduce((acc, v) => acc + v, 0);
  const grossLossAbs = Math.abs(
    grossPnLList.filter((v) => v < 0).reduce((acc, v) => acc + v, 0)
  );
  const bestTrade = Math.max(...netProfits);
  const worstTrade = Math.min(...netProfits);

  const fx = (value) => {
    const sign = value >= 0 ? '+' : '';
    return `${sign}${value.toFixed(2)}`;
  };

  const valueClass = (value) =>
    value > 0 ? 'positive' : value < 0 ? 'negative' : '';

  const isZh = (navigator.language || '').startsWith('zh');

  const makeRow = (labelEn, labelZh, value, cls) => ({
    label: isZh ? labelZh : labelEn,
    value,
    cls: cls || '',
  });

  const rows = [
    makeRow('Total Trades', '总交易数', totalTrades),
    makeRow('Win Rate', '胜率', `${winRate.toFixed(1)}%`),
    makeRow('Winning Trades', '盈利笔数', winTrades),
    makeRow('Losing Trades', '亏损笔数', lossTrades),
    makeRow('Fee', '手续费', `${totalFee.toFixed(2)}`, ''),
    makeRow(
      'Profit Factor',
      '盈亏比',
      profitLossRatio == null ? '∞' : profitLossRatio.toFixed(2),
      ''
    ),
    makeRow('Net PnL', '总收益', fx(totalProfit), valueClass(totalProfit)),
    makeRow(
      'Average PnL per Trade',
      '平均单笔收益',
      fx(avgProfit),
      valueClass(avgProfit)
    ),
    makeRow('Best Trade', '最佳单笔', fx(bestTrade), valueClass(bestTrade)),
    makeRow('Worst Trade', '最差单笔', fx(worstTrade), valueClass(worstTrade)),
    makeRow('Total Net Profit', '净盈利', netGrossProfit.toFixed(2), 'positive'),
    makeRow(
      'Total Net Loss',
      '净亏损',
      netGrossLossAbs.toFixed(2),
      'negative'
    ),
    makeRow('Gross Profit', '毛盈利', grossProfit.toFixed(2), 'positive'),
    makeRow('Gross Loss', '毛亏损', grossLossAbs.toFixed(2), 'negative'),
  ];

  return { totalTrades, rows };
}
