import React, { createContext, useContext, useState, useMemo, useCallback, useEffect } from 'react';

const TradingDataContext = createContext(null);

export function TradingDataProvider({ children }) {
  // --- Source data (from window globals set by backend or polling) ---
  const [dataSourceList] = useState(() => window.dataSourceList || []);
  const [historyPositionList, setHistoryPositionList] = useState(() => {
    const arr = window.historyPositionList || [];
    return Array.isArray(arr) ? [...arr] : [];
  });
  const [historyOrderList, setHistoryOrderList] = useState(() => {
    const arr = window.historyOrderList || [];
    return Array.isArray(arr) ? [...arr] : [];
  });

  // --- UI state with localStorage persistence ---
  const [currentSymbol, setCurrentSymbol] = useState(() => {
    const symbols = [
      ...new Set((window.dataSourceList || []).map((v) => v.metadata.symbol)),
    ];
    return symbols[0] || '';
  });
  const [currentLevel, setCurrentLevel] = useState(() => {
    const levels = [
      ...new Set((window.dataSourceList || []).map((v) => v.metadata.level)),
    ];
    return levels[0] || '';
  });
  const [theme, setThemeState] = useState(() => {
    const saved = localStorage.getItem('theme');
    return saved || 'dark';
  });
  const [magnet, setMagnetState] = useState(() => {
    const saved = localStorage.getItem('magnet');
    return saved !== 'false';
  });
  const [showVolume, setShowVolumeState] = useState(() => {
    const saved = localStorage.getItem('showVolume');
    return saved !== 'false';
  });
  const locale = typeof navigator !== 'undefined' ? navigator.language || 'en-US' : 'en-US';

  // --- Derived state ---
  const symbolList = useMemo(
    () => [...new Set(dataSourceList.map((v) => v.metadata.symbol))],
    [dataSourceList]
  );
  const levelList = useMemo(
    () => [...new Set(dataSourceList.map((v) => v.metadata.level))],
    [dataSourceList]
  );
  const currentDataSource = useMemo(
    () =>
      dataSourceList.find(
        (v) =>
          v.metadata.symbol === currentSymbol &&
          v.metadata.level === currentLevel
      ) ||
      dataSourceList.find((v) => v.metadata.symbol === currentSymbol) ||
      null,
    [dataSourceList, currentSymbol, currentLevel]
  );
  const isZh = useMemo(() => locale.startsWith('zh'), [locale]);

  // --- Sync to window globals for backward compat ---
  useEffect(() => {
    window.theme = theme;
    window.locale = locale;
    window.magnet = magnet;
    window.showVolume = showVolume;
  }, [theme, locale, magnet, showVolume]);

  useEffect(() => {
    if (currentDataSource) {
      window.dataSource = currentDataSource;
    }
  }, [currentDataSource]);

  useEffect(() => {
    window.historyPositionList = historyPositionList;
  }, [historyPositionList]);

  useEffect(() => {
    window.historyOrderList = historyOrderList;
  }, [historyOrderList]);

  // --- Theme body class ---
  useEffect(() => {
    const body = document.body;
    body.classList.remove('theme-dark', 'theme-light');
    body.classList.add(`theme-${theme}`);
  }, [theme]);

  // --- Persist settings to localStorage ---
  const setTheme = useCallback((newTheme) => {
    // Update body class synchronously BEFORE state update,
    // so child effects (chart) read the new CSS variables.
    const body = document.body;
    body.classList.remove('theme-dark', 'theme-light');
    body.classList.add(`theme-${newTheme}`);
    setThemeState(newTheme);
    localStorage.setItem('theme', newTheme);
  }, []);

  const setMagnet = useCallback((val) => {
    setMagnetState(val);
    localStorage.setItem('magnet', val);
  }, []);

  const setShowVolume = useCallback((val) => {
    setShowVolumeState(val);
    localStorage.setItem('showVolume', val);
  }, []);

  // --- Refresh function (called after polling eval) ---
  const refreshData = useCallback(() => {
    if (window.historyPositionList) {
      setHistoryPositionList([...window.historyPositionList]);
    }
    if (window.historyOrderList) {
      setHistoryOrderList([...window.historyOrderList]);
    }
  }, []);

  const value = useMemo(
    () => ({
      dataSourceList,
      historyPositionList,
      historyOrderList,
      currentSymbol,
      currentLevel,
      theme,
      magnet,
      showVolume,
      locale,
      isZh,
      symbolList,
      levelList,
      currentDataSource,
      setCurrentSymbol,
      setCurrentLevel,
      setTheme,
      setMagnet,
      setShowVolume,
      refreshData,
    }),
    [
      dataSourceList,
      historyPositionList,
      historyOrderList,
      currentSymbol,
      currentLevel,
      theme,
      magnet,
      showVolume,
      locale,
      isZh,
      symbolList,
      levelList,
      currentDataSource,
      setCurrentSymbol,
      setCurrentLevel,
      setTheme,
      setMagnet,
      setShowVolume,
      refreshData,
    ]
  );

  return (
    <TradingDataContext.Provider value={value}>
      {children}
    </TradingDataContext.Provider>
  );
}

export function useTradingData() {
  const ctx = useContext(TradingDataContext);
  if (!ctx) {
    throw new Error('useTradingData must be used within TradingDataProvider');
  }
  return ctx;
}
