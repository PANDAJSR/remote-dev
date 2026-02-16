import { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import './Terminal.css';

function TerminalPage() {
  const terminalRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    if (!terminalRef.current || !containerRef.current) return;

    // 创建终端
    const term = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: 'Menlo, Monaco, "Courier New", monospace',
      theme: {
        background: '#1e1e1e',
        foreground: '#d4d4d4',
      },
      allowProposedApi: true,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    term.open(terminalRef.current);
    fitAddon.fit();

    termRef.current = term;
    fitAddonRef.current = fitAddon;

    // 连接 WebSocket
    const ws = new WebSocket(`ws://${window.location.host}/ws`);
    wsRef.current = ws;

    ws.onopen = () => {
      console.log('WebSocket connected');
      term.writeln('\r\n\x1b[32mConnected to terminal server\x1b[0m\r\n');
    };

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === 'output') {
        term.write(msg.data);
      }
    };

    ws.onclose = () => {
      console.log('WebSocket disconnected');
      term.writeln('\r\n\x1b[31mDisconnected from server\x1b[0m');
    };

    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      term.writeln('\r\n\x1b[31mConnection error\x1b[0m');
    };

    // 用户输入处理
    term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'input', data }));
      }
    });

    // 终端大小变化时调整
    let resizeTimeout: ReturnType<typeof setTimeout> | null = null;
    const handleResize = () => {
      if (resizeTimeout) clearTimeout(resizeTimeout);
      resizeTimeout = setTimeout(() => {
        fitAddon.fit();
        const { cols, rows } = term;
        console.log('Terminal resized:', cols, rows);
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: 'resize', cols, rows }));
        }
      }, 100);
    };

    window.addEventListener('resize', handleResize);

    // 使用 ResizeObserver 监听终端容器大小变化（用于 Dockview 面板调整）
    let resizeObserver: ResizeObserver | null = null;

    // 延迟初始化 ResizeObserver，确保 DOM 完全渲染
    const initObserver = setTimeout(() => {
      const container = containerRef.current;
      if (container && 'ResizeObserver' in window) {
        resizeObserver = new ResizeObserver((entries) => {
          console.log('ResizeObserver triggered:', entries[0]?.contentRect);
          handleResize();
        });
        resizeObserver.observe(container);
        console.log('ResizeObserver attached to container');
      }
    }, 500);

    // 初始调整
    setTimeout(handleResize, 100);

    return () => {
      window.removeEventListener('resize', handleResize);
      clearTimeout(initObserver);
      if (resizeTimeout) clearTimeout(resizeTimeout);
      if (resizeObserver) {
        resizeObserver.disconnect();
      }
      ws.close();
      term.dispose();
    };
  }, []);

  return (
    <div className="app">
      <div ref={containerRef} className="terminal-container">
        <div ref={terminalRef} className="terminal" />
      </div>
    </div>
  );
}

export default TerminalPage;
