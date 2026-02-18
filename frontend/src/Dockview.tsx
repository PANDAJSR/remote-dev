import { useMemo, useRef, useCallback, useState } from 'react';
import {
  DockviewReact,
  DockviewReadyEvent,
  IDockviewPanelProps,
  DockviewApi,
} from 'dockview-react';
import 'dockview-react/dist/styles/dockview.css';
import './Dockview.css';
import TerminalPage from './Terminal';
import FileExplorer, { FileExplorerRef } from './FileExplorer';
import EditorPanel from './EditorPanel';
import MenuBar from './MenuBar';
import RemoteDesktopPanel from './RemoteDesktop';

// 终端面板组件
const TerminalPanel = () => {
  return (
    <div className="terminal-panel">
      <TerminalPage />
    </div>
  );
};

// 编辑器面板组件包装器
const EditorPanelWrapper = (props: IDockviewPanelProps<{ filePath: string; fileName: string; onEditorMount?: (api: any) => void; onEditorUnmount?: () => void }>) => {
  return (
    <div className="editor-panel-wrapper">
      <EditorPanel 
        filePath={props.params.filePath} 
        fileName={props.params.fileName} 
        onEditorMount={props.params.onEditorMount}
        onEditorUnmount={props.params.onEditorUnmount}
      />
    </div>
  );
};

// 远程桌面面板组件
const RemoteDesktopPanelWrapper = () => {
  return (
    <div className="remote-desktop-panel-wrapper" style={{ height: '100%', width: '100%' }}>
      <RemoteDesktopPanel params={{}} />
    </div>
  );
};

const DockviewApp = () => {
  const apiRef = useRef<DockviewApi | null>(null);
  const openEditorsRef = useRef<Set<string>>(new Set());
  const [activeEditorApi, setActiveEditorApi] = useState<any>(null);
  
  // 编辑器 API ref，用于存储所有打开的编辑器
  const editorApisRef = useRef<Map<string, any>>(new Map());
  
  // FileExplorer ref，用于调用新建文件方法
  const fileExplorerRef = useRef<FileExplorerRef | null>(null);

  const onReady = useCallback((event: DockviewReadyEvent) => {
    apiRef.current = event.api;

    // 添加文件资源管理器面板（左侧）
    event.api.addPanel({
      id: 'fileExplorer',
      component: 'fileExplorer',
      title: '文件管理',
    });

    // 添加终端面板（右侧/主区域）
    event.api.addPanel({
      id: 'terminal',
      component: 'terminal',
      title: '终端',
    });

    // 添加远程桌面面板
    event.api.addPanel({
      id: 'remote-desktop',
      component: 'remoteDesktop',
      title: '远程桌面',
      position: { referencePanel: 'terminal', direction: 'right' },
    });
  }, []);

  // 创建文件资源管理器面板组件，传入打开文件的回调
  const FileExplorerPanelWithCallback = useCallback(() => {
    const handleFileOpen = (filePath: string, fileName: string) => {
      if (!apiRef.current) return;

      const panelId = `editor-${filePath}`;

      // 检查文件是否已经打开
      if (openEditorsRef.current.has(filePath)) {
        // 聚焦到已存在的面板
        const existingPanel = apiRef.current.getPanel(panelId);
        if (existingPanel) {
          existingPanel.api.setActive();
          return;
        }
      }

      // 创建编辑器挂载回调
      const handleEditorMount = (editorApi: any) => {
        editorApisRef.current.set(panelId, editorApi);
        setActiveEditorApi(editorApi);
      };

      const handleEditorUnmount = () => {
        editorApisRef.current.delete(panelId);
        // 如果还有其他编辑器，切换到最新的一个
        const remainingEditors = Array.from(editorApisRef.current.values());
        setActiveEditorApi(remainingEditors.length > 0 ? remainingEditors[remainingEditors.length - 1] : null);
        openEditorsRef.current.delete(filePath);
      };

      // 添加新面板
      const panel = apiRef.current.addPanel({
        id: panelId,
        component: 'editor',
        title: fileName,
        params: {
          filePath,
          fileName,
          onEditorMount: handleEditorMount,
          onEditorUnmount: handleEditorUnmount,
        },
      });

      openEditorsRef.current.add(filePath);

      // 监听面板激活事件
      panel.api.onDidActiveChange((event) => {
        if (event.isActive) {
          const api = editorApisRef.current.get(panelId);
          if (api) {
            setActiveEditorApi(api);
          }
        }
      });

      // 监听面板关闭事件
      panel.api.onDidVisibilityChange((visible) => {
        if (!visible) {
          setTimeout(() => {
            const p = apiRef.current?.getPanel(panelId);
            if (!p) {
              openEditorsRef.current.delete(filePath);
              editorApisRef.current.delete(panelId);
            }
          }, 100);
        }
      });
    };

    return (
      <div className="file-explorer-panel">
        <FileExplorer ref={fileExplorerRef} onFileOpen={handleFileOpen} />
      </div>
    );
  }, []);

  const components = useMemo(
    () => ({
      terminal: TerminalPanel,
      editor: EditorPanelWrapper,
      fileExplorer: FileExplorerPanelWithCallback,
      remoteDesktop: RemoteDesktopPanelWrapper,
    }),
    [FileExplorerPanelWithCallback]
  );

  // 菜单操作处理函数
  const handleNewFile = useCallback(() => {
    // 调用 FileExplorer 的新建文件方法
    // 这会在根目录创建一个新文件项并进入重命名状态
    fileExplorerRef.current?.createNewFile();
  }, []);

  const handleSave = useCallback(() => {
    // 触发保存快捷键到当前活动编辑器
    if (activeEditorApi) {
      activeEditorApi.trigger('menu', 'editor.action.save');
    }
  }, [activeEditorApi]);

  const handleOpenFolder = useCallback((path: string) => {
    // 调用 FileExplorer 的加载文件夹方法
    fileExplorerRef.current?.loadFolder(path);
  }, []);

  return (
    <div className="app-container">
      <MenuBar 
        onNewFile={handleNewFile}
        onSave={handleSave}
        onOpenFolder={handleOpenFolder}
        activeEditor={activeEditorApi}
      />
      <div className="dockview-wrapper">
        <DockviewReact
          components={components}
          onReady={onReady}
          className="dockview-theme-dark"
        />
      </div>
    </div>
  );
};

export default DockviewApp;
