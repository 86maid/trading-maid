import React, { useState, useCallback, useRef } from 'react';
import { ConfigProvider, theme as antdTheme } from 'antd';
import { TradingDataProvider, useTradingData } from './context/TradingDataContext';
import { usePolling } from './hooks/usePolling';
import Toolbar from './components/Toolbar';
import ChartPanel from './components/ChartPanel';
import HistoryPanel from './components/HistoryPanel';

function AppContent() {
  const { theme, refreshData } = useTradingData();
  const [scrollToTime, setScrollToTime] = useState(null);
  const [activeTab, setActiveTab] = useState('summary');
  const historyPanelRef = useRef(null);

  // Start polling
  usePolling(refreshData);

  const handleChartReady = useCallback((scrollFn) => {
    setScrollToTime(() => scrollFn);
  }, []);

  const handleTabChange = useCallback((key) => {
    setActiveTab(key);
  }, []);

  return (
    <ConfigProvider
      theme={{
        algorithm:
          theme === 'dark'
            ? antdTheme.darkAlgorithm
            : antdTheme.defaultAlgorithm,
        token: {
          borderRadius: 8,
        },
      }}
    >
      <Toolbar />
      <div className="ab">
        <ChartPanel
          onChartReady={handleChartReady}
          historyPanelRef={historyPanelRef}
        />
        {scrollToTime && (
          <HistoryPanel
            ref={historyPanelRef}
            scrollToTime={scrollToTime}
            activeTab={activeTab}
            onTabChange={handleTabChange}
          />
        )}
      </div>
    </ConfigProvider>
  );
}

export default function App() {
  return (
    <TradingDataProvider>
      <AppContent />
    </TradingDataProvider>
  );
}
