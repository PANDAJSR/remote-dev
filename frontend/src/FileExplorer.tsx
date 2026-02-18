import { useEffect, useState, useCallback, useRef, forwardRef, useImperativeHandle } from 'react';
import {
  ControlledTreeEnvironment,
  Tree,
  TreeItem,
  TreeRef,
  TreeItemIndex,
} from 'react-complex-tree';
import 'react-complex-tree/lib/style-modern.css';
import './FileExplorer.css';
import { ContextMenu, ContextMenuItem } from './ContextMenu';

// 文件变更事件类型
interface FileChangeEvent {
  event: 'created' | 'modified' | 'deleted' | 'renamed';
  path: string;
  name: string;
  is_directory?: boolean;
  old_path?: string;
  new_path?: string;
}

// WebSocket 消息类型
interface FileWatchMessage {
  type: 'file_change';
  event: FileChangeEvent['event'];
  path: string;
  name: string;
  is_directory?: boolean;
  old_path?: string;
  new_path?: string;
}

// 标准化路径（统一使用正斜杠，并确保一致性）
const normalizePath = (path: string): string => {
  return path.replace(/\\/g, '/');
};

// 路径比较（Windows 上不区分大小写）
const pathsEqual = (path1: string, path2: string): boolean => {
  const normalized1 = normalizePath(path1);
  const normalized2 = normalizePath(path2);
  // Windows 路径不区分大小写
  if (window.navigator.platform.toLowerCase().includes('win')) {
    return normalized1.toLowerCase() === normalized2.toLowerCase();
  }
  return normalized1 === normalized2;
};

interface FileEntry {
  name: string;
  path: string;
  is_directory: boolean;
  children?: FileEntry[];
}

interface FileExplorerProps {
  onFileOpen?: (filePath: string, fileName: string) => void;
}

export interface FileExplorerRef {
  createNewFile: () => void;
  loadFolder: (path: string) => void;
}

interface FileTreeItem extends TreeItem<string> {
  path: string;
  isNewFile?: boolean;
  isNewFolder?: boolean;
}

interface ClipboardItem {
  path: string;
  name: string;
  isFolder: boolean;
}

interface ContextMenuState {
  visible: boolean;
  x: number;
  y: number;
  itemId: string | null;
}

const FileExplorer = forwardRef<FileExplorerRef, FileExplorerProps>(({ onFileOpen }, ref) => {
  const [items, setItems] = useState<Record<TreeItemIndex, FileTreeItem>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [focusedItem, setFocusedItem] = useState<TreeItemIndex>('root');
  const [expandedItems, setExpandedItems] = useState<TreeItemIndex[]>(['root']);
  const [selectedItems, setSelectedItems] = useState<TreeItemIndex[]>([]);
  const treeRef = useRef<TreeRef<string> | null>(null);
  const newFileIdRef = useRef<string | null>(null);
  const rootPathRef = useRef<string>('');
  const itemsRef = useRef<Record<TreeItemIndex, FileTreeItem>>({});

  // 虚拟剪贴板状态
  const [clipboard, setClipboard] = useState<ClipboardItem | null>(null);

  // 右键菜单状态
  const [contextMenu, setContextMenu] = useState<ContextMenuState>({
    visible: false,
    x: 0,
    y: 0,
    itemId: null,
  });

  // WebSocket 状态
  const [wsConnected, setWsConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const wsReconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const wsConnectingRef = useRef<boolean>(false);
  const wsIntentionalCloseRef = useRef<boolean>(false);
  const wsAttemptRef = useRef<number>(0);
  const subscribedPathsRef = useRef<Set<string>>(new Set());
  const WS_RECONNECT_DELAY = 3000;

  // 发送订阅请求到后端
  const subscribeToPath = useCallback((path: string) => {
    if (wsRef.current?.readyState === WebSocket.OPEN && !subscribedPathsRef.current.has(path)) {
      wsRef.current.send(JSON.stringify({ path }));
      subscribedPathsRef.current.add(path);
      console.log('Subscribed to path:', path);
    }
  }, []);

  // 加载文件树
  const loadTree = useCallback(async (customPath?: string) => {
    setLoading(true);
    setError(null);
    try {
      const url = customPath
        ? `/api/tree?path=${encodeURIComponent(customPath)}`
        : '/api/tree';
      const response = await fetch(url);
      const data: FileEntry = await response.json();
      const newItems: Record<TreeItemIndex, FileTreeItem> = {};

      const loadEntry = (entry: FileEntry, parentId: string): string => {
        const id = parentId ? `${parentId}/${entry.name}` : 'root';
        const children: string[] = [];
        if (entry.children) {
          entry.children.forEach(child => {
            const childId = loadEntry(child, id);
            children.push(childId);
          });
        }
        newItems[id] = {
          index: id,
          data: entry.name,
          path: entry.path,
          isFolder: entry.is_directory,
          children: entry.is_directory ? children : undefined,
          canMove: false,
          canRename: !entry.is_directory,
        };
        return id;
      };

      loadEntry(data, '');
      itemsRef.current = newItems;
      setItems(newItems);
      rootPathRef.current = newItems['root']?.path || '';
      setLoading(false);
      // 重置状态
      setFocusedItem('root');
      setExpandedItems(['root']);
      setSelectedItems([]);

      // 订阅新加载的路径以接收文件变更通知
      const rootPath = newItems['root']?.path;
      if (rootPath) {
        subscribeToPath(rootPath);
      }
    } catch (err: any) {
      console.error('Error fetching directory tree:', err);
      setError(err.message);
      setLoading(false);
    }
  }, [subscribeToPath]);

  // 检查路径是否在当前文件树范围内
  const isPathInCurrentTree = useCallback((filePath: string): boolean => {
    const rootPath = rootPathRef.current;
    if (!rootPath) return false;

    const normalizedFilePath = normalizePath(filePath);
    const normalizedRootPath = normalizePath(rootPath);

    // 处理 Windows 根路径特殊情况：当前端显示的是 "/"（Windows 驱动器根目录）时，
    // 后端返回的文件路径可能是 "D:/..." 完整路径格式
    // 这种情况下，任何绝对路径都应该被认为在当前树范围内
    if (normalizedRootPath === '/' || normalizedRootPath.match(/^[A-Za-z]:$/)) {
      // 如果是根路径，检查文件路径是否是绝对路径（Windows: D:/... 或 Unix: /...）
      return normalizedFilePath.startsWith('/') ||
             /^[A-Za-z]:/.test(normalizedFilePath);
    }

    // 文件路径应该是根路径的子路径
    return normalizedFilePath.startsWith(normalizedRootPath + '/') ||
           normalizedFilePath === normalizedRootPath;
  }, []);

  // 处理文件创建事件
  const handleFileCreated = useCallback((path: string, name: string, isDirectory: boolean) => {
    // 标准化传入的路径
    const normalizedFilePath = normalizePath(path);
    console.log('handleFileCreated - path:', path, 'normalized:', normalizedFilePath, 'name:', name);

    // 检查文件是否在当前显示的文件树范围内
    if (!isPathInCurrentTree(normalizedFilePath)) {
      console.log('File created outside current tree, ignoring:', normalizedFilePath);
      return;
    }

    // 查找父目录（只查找已加载的，不动态创建）
    const lastSlashIndex = normalizedFilePath.lastIndexOf('/');
    const parentPath = lastSlashIndex > 0 ? normalizedFilePath.substring(0, lastSlashIndex) : '';
    let parentId: string | null = null;
    
    if (!parentPath) {
      parentId = 'root';
    } else {
      // 查找已加载的父目录
      for (const [id, item] of Object.entries(itemsRef.current)) {
        if (pathsEqual(item.path, parentPath)) {
          parentId = id;
          break;
        }
      }
    }
    
    if (!parentId) {
      console.log('Parent directory not loaded, skipping created file:', normalizedFilePath);
      return;
    }
    
    // 检查父目录是否已展开（有 children 属性）
    let parentItem = itemsRef.current[parentId];
    if (!parentItem || (!parentItem.children && parentId !== 'root')) {
      console.log('Parent directory not expanded, skipping created file:', normalizedFilePath);
      return;
    }

    const newId = `${parentId}/${name}`;

    // 检查是否已存在
    if (itemsRef.current[newId]) {
      console.log('Item already exists:', newId);
      return;
    }

    const newItems = { ...itemsRef.current };

    // 添加新项 - 使用标准化路径
    newItems[newId] = {
      index: newId,
      data: name,
      path: normalizedFilePath,
      isFolder: isDirectory,
      children: isDirectory ? [] : undefined,
      canMove: false,
      canRename: !isDirectory,
    };
    console.log('Created new item:', newId, 'with path:', normalizedFilePath);

    // 更新父目录的 children
    parentItem = newItems[parentId];
    if (parentItem) {
      const existingChildren = parentItem.children || [];
      // 按文件夹优先、字母顺序插入
      const insertIndex = existingChildren.findIndex(childId => {
        const child = newItems[childId];
        if (!child) return false;
        if (isDirectory && !child.isFolder) return true;
        if (!isDirectory && child.isFolder) return false;
        return child.data.localeCompare(name) > 0;
      });

      if (insertIndex === -1) {
        newItems[parentId] = {
          ...parentItem,
          children: [...existingChildren, newId],
        };
      } else {
        const newChildren = [...existingChildren];
        newChildren.splice(insertIndex, 0, newId);
        newItems[parentId] = {
          ...parentItem,
          children: newChildren,
        };
      }
    }

    itemsRef.current = newItems;
    setItems(newItems);
  }, [isPathInCurrentTree]);

  // 处理文件删除事件
  const handleFileDeleted = useCallback((path: string, _name: string) => {
    // 查找对应的项 ID（使用标准化路径比较）
    const normalizedTargetPath = normalizePath(path);
    console.log('handleFileDeleted - path:', path, 'normalized:', normalizedTargetPath);

    // 检查文件是否在当前显示的文件树范围内
    if (!isPathInCurrentTree(normalizedTargetPath)) {
      console.log('File deleted outside current tree, ignoring:', normalizedTargetPath);
      return;
    }

    let itemIdToDelete: string | null = null;
    for (const [id, item] of Object.entries(itemsRef.current)) {
      if (pathsEqual(item.path, normalizedTargetPath)) {
        itemIdToDelete = id;
        break;
      }
    }

    if (!itemIdToDelete) {
      console.log('Item not found for deletion:', normalizedTargetPath);
      console.log('Available paths:', Object.entries(itemsRef.current).map(([id, item]) => `${id}: ${item.path}`).join(', '));
      return;
    }
    console.log('Found item to delete:', itemIdToDelete);

    const newItems = { ...itemsRef.current };

    // 从父目录的 children 中移除
    const parentId = itemIdToDelete.substring(0, itemIdToDelete.lastIndexOf('/')) || 'root';
    const parentItem = newItems[parentId];
    if (parentItem && parentItem.children) {
      newItems[parentId] = {
        ...parentItem,
        children: parentItem.children.filter(id => id !== itemIdToDelete),
      };
    }

    // 删除该项及其所有子项
    const deleteRecursively = (id: string) => {
      const item = newItems[id];
      if (item?.children) {
        item.children.forEach(childId => deleteRecursively(String(childId)));
      }
      delete newItems[id];
    };
    deleteRecursively(itemIdToDelete);

    itemsRef.current = newItems;
    setItems(newItems);

    // 如果被删除的项是当前选中的，重置选中状态
    if (selectedItems.includes(itemIdToDelete)) {
      setSelectedItems(prev => prev.filter(id => id !== itemIdToDelete));
    }
    if (focusedItem === itemIdToDelete) {
      setFocusedItem('root');
    }
  }, [selectedItems, focusedItem, isPathInCurrentTree]);

  // 处理文件重命名事件
  const handleFileRenamed = useCallback((oldPath: string, newPath: string, name: string, _isDirectory?: boolean) => {
    // 查找旧路径对应的项（使用标准化路径比较）
    const normalizedOldPath = normalizePath(oldPath);
    const normalizedNewPath = normalizePath(newPath);
    console.log('handleFileRenamed - oldPath:', oldPath, 'newPath:', newPath);
    console.log('normalized - old:', normalizedOldPath, 'new:', normalizedNewPath);

    // 检查文件是否在当前显示的文件树范围内
    if (!isPathInCurrentTree(normalizedOldPath) && !isPathInCurrentTree(normalizedNewPath)) {
      console.log('File renamed outside current tree, ignoring:', normalizedOldPath, '->', normalizedNewPath);
      return;
    }

    let oldItemId: string | null = null;
    for (const [id, item] of Object.entries(itemsRef.current)) {
      if (pathsEqual(item.path, normalizedOldPath)) {
        oldItemId = id;
        break;
      }
    }

    if (!oldItemId) {
      console.log('Item not found for rename:', normalizedOldPath);
      return;
    }
    console.log('Found item to rename:', oldItemId);

    const newItems = { ...itemsRef.current };
    const oldItem = newItems[oldItemId];

    // 生成新的 ID
    const parentId = oldItemId.substring(0, oldItemId.lastIndexOf('/')) || 'root';
    const newId = `${parentId}/${name}`;

    // 更新项的 ID 和路径
    newItems[newId] = {
      ...oldItem,
      index: newId,
      data: name,
      path: normalizedNewPath,
    };

    // 更新父目录的 children
    const parentItem = newItems[parentId];
    if (parentItem && parentItem.children) {
      newItems[parentId] = {
        ...parentItem,
        children: parentItem.children.map(id => id === oldItemId ? newId : id),
      };
    }

    // 递归更新所有子项的路径
    const updateChildrenPaths = (oldParentId: string, newParentId: string, oldBasePath: string, newBasePath: string) => {
      const parent = newItems[oldParentId];
      if (!parent?.children) return;

      parent.children.forEach(childId => {
        const childIdStr = String(childId);
        const child = newItems[childIdStr];
        if (child) {
          const newChildPath = child.path.replace(oldBasePath, newBasePath);
          const newChildId = childIdStr.replace(oldParentId, newParentId);

          newItems[newChildId] = {
            ...child,
            index: newChildId,
            path: newChildPath,
          };

          delete newItems[childIdStr];

          // 递归更新子项
          updateChildrenPaths(childIdStr, newChildId, oldBasePath, newBasePath);
        }
      });
    };

    if (oldItem.isFolder) {
      updateChildrenPaths(oldItemId, newId, normalizedOldPath, normalizedNewPath);
    }

    // 删除旧 ID
    delete newItems[oldItemId];

    itemsRef.current = newItems;
    setItems(newItems);

    // 更新选中状态
    if (selectedItems.includes(oldItemId)) {
      setSelectedItems(prev => prev.map(id => id === oldItemId ? newId : id));
    }
    if (focusedItem === oldItemId) {
      setFocusedItem(newId);
    }
  }, [selectedItems, focusedItem, isPathInCurrentTree]);

  // 处理 WebSocket 消息
  const handleWebSocketMessage = useCallback((event: MessageEvent) => {
    try {
      const data: FileWatchMessage = JSON.parse(event.data);

      if (data.type === 'file_change') {
        console.log('File change event:', data);

        switch (data.event) {
          case 'created':
            handleFileCreated(data.path, data.name, data.is_directory || false);
            break;
          case 'deleted':
            handleFileDeleted(data.path, data.name);
            break;
          case 'renamed':
            if (data.old_path && data.new_path) {
              handleFileRenamed(data.old_path, data.new_path, data.name, data.is_directory);
            }
            break;
          case 'modified':
            // 文件内容修改，不需要更新文件树结构
            console.log('File modified:', data.path);
            break;
        }
      }
    } catch (err) {
      console.error('Error parsing WebSocket message:', err);
    }
  }, [handleFileCreated, handleFileDeleted, handleFileRenamed]);

  // 建立 WebSocket 连接
  const connectWebSocket = useCallback(() => {
    // 防止重复连接
    if (wsConnectingRef.current || (wsRef.current?.readyState === WebSocket.CONNECTING)) {
      console.log('WebSocket connection already in progress, skipping');
      return;
    }

    // 如果已有连接，不要重复连接
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      console.log('WebSocket already connected');
      return;
    }

    // 清理之前的连接
    if (wsRef.current) {
      wsIntentionalCloseRef.current = true;
      wsRef.current.close();
    }

    if (wsReconnectTimeoutRef.current) {
      clearTimeout(wsReconnectTimeoutRef.current);
      wsReconnectTimeoutRef.current = null;
    }

    wsConnectingRef.current = true;
    wsIntentionalCloseRef.current = false;

    // 判断使用哪个 WebSocket URL
    // 第1次尝试：使用相对路径（通过 Vite 代理，适用于 localhost 访问）
    // 第2次尝试：直接连接后端 3000 端口（适用于 IP 访问开发服务器）
    const attempt = wsAttemptRef.current % 2;
    let wsUrl: string;

    if (attempt === 0) {
      // 相对路径，通过 Vite 代理
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      wsUrl = `${protocol}//${window.location.host}/ws/files`;
    } else {
      // 直接连接后端端口
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const hostname = window.location.hostname;
      wsUrl = `${protocol}//${hostname}:3000/ws/files`;
    }

    console.log(`Connecting to WebSocket (attempt ${attempt + 1}):`, wsUrl);
    wsAttemptRef.current++;

    const ws = new WebSocket(wsUrl);

    ws.onopen = () => {
      console.log('WebSocket connected');
      setWsConnected(true);
      wsConnectingRef.current = false;
      wsAttemptRef.current = 0; // 重置尝试计数

      // 清空之前的订阅记录，只订阅当前根路径
      // 避免在切换目录后收到旧目录路径的文件变更事件
      subscribedPathsRef.current.clear();

      // 订阅当前根路径
      if (rootPathRef.current) {
        subscribeToPath(rootPathRef.current);
      }
    };

    ws.onmessage = handleWebSocketMessage;

    ws.onclose = (event) => {
      console.log('WebSocket disconnected, code:', event.code, 'intentional:', wsIntentionalCloseRef.current);
      setWsConnected(false);
      wsConnectingRef.current = false;
      wsRef.current = null;

      // 如果是预期的关闭（组件卸载），不重连
      if (wsIntentionalCloseRef.current) {
        console.log('Intentional close, not reconnecting');
        return;
      }

      // 自动重连
      wsReconnectTimeoutRef.current = setTimeout(() => {
        console.log('Attempting to reconnect WebSocket...');
        connectWebSocket();
      }, WS_RECONNECT_DELAY);
    };

    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      wsConnectingRef.current = false;
    };

    wsRef.current = ws;
  }, [handleWebSocketMessage]);

  // 断开 WebSocket 连接
  const disconnectWebSocket = useCallback(() => {
    wsIntentionalCloseRef.current = true;
    if (wsReconnectTimeoutRef.current) {
      clearTimeout(wsReconnectTimeoutRef.current);
      wsReconnectTimeoutRef.current = null;
    }
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    setWsConnected(false);
    wsConnectingRef.current = false;
  }, []);

  // 创建新文件功能
  const createNewFile = useCallback((targetItemId?: string) => {
    if (!treeRef.current) return;

    const timestamp = Date.now();
    const targetId = targetItemId || 'root';
    
    // 获取目标目录信息
    const targetItem = itemsRef.current[targetId];
    let parentId: string;
    let parentPath: string;
    let tempId: string;

    if (!targetItem) {
      // 如果目标项不存在，使用根目录
      parentId = 'root';
      parentPath = rootPathRef.current;
      tempId = `root/__NEW_FILE_${timestamp}__`;
    } else if (targetItem.isFolder) {
      // 如果右键点击的是文件夹，在文件夹内创建
      parentId = targetId;
      parentPath = targetItem.path;
      tempId = `${targetId}/__NEW_FILE_${timestamp}__`;
    } else {
      // 如果右键点击的是文件，在文件所在目录创建
      parentId = targetId.substring(0, targetId.lastIndexOf('/')) || 'root';
      const parentItem = itemsRef.current[parentId];
      parentPath = parentItem ? parentItem.path : rootPathRef.current;
      tempId = `${parentId}/__NEW_FILE_${timestamp}__`;
    }

    const newItems = { ...itemsRef.current };
    newItems[tempId] = {
      index: tempId,
      data: '',
      path: `${parentPath}/newfile`,
      isFolder: false,
      children: undefined,
      canMove: false,
      canRename: true,
      isNewFile: true,
    };

    const parentItem = newItems[parentId];
    if (parentItem) {
      newItems[parentId] = {
        ...parentItem,
        children: [...(parentItem.children || []), tempId],
      };
    }

    itemsRef.current = newItems;
    setItems(newItems);
    newFileIdRef.current = tempId;

    // 确保目标目录展开并聚焦到新项
    setExpandedItems(prev => [...new Set([...prev, parentId])]);
    setFocusedItem(tempId);

    setTimeout(() => {
      treeRef.current?.startRenamingItem?.(tempId);
    }, 100);
  }, []);

  // 创建新文件夹功能
  const createNewFolder = useCallback((targetItemId?: string) => {
    if (!treeRef.current) return;

    const timestamp = Date.now();
    const targetId = targetItemId || 'root';
    
    // 获取目标目录信息
    const targetItem = itemsRef.current[targetId];
    let parentId: string;
    let parentPath: string;
    let tempId: string;

    if (!targetItem) {
      // 如果目标项不存在，使用根目录
      parentId = 'root';
      parentPath = rootPathRef.current;
      tempId = `root/__NEW_FOLDER_${timestamp}__`;
    } else if (targetItem.isFolder) {
      // 如果右键点击的是文件夹，在文件夹内创建
      parentId = targetId;
      parentPath = targetItem.path;
      tempId = `${targetId}/__NEW_FOLDER_${timestamp}__`;
    } else {
      // 如果右键点击的是文件，在文件所在目录创建
      parentId = targetId.substring(0, targetId.lastIndexOf('/')) || 'root';
      const parentItem = itemsRef.current[parentId];
      parentPath = parentItem ? parentItem.path : rootPathRef.current;
      tempId = `${parentId}/__NEW_FOLDER_${timestamp}__`;
    }

    const newItems = { ...itemsRef.current };
    newItems[tempId] = {
      index: tempId,
      data: '',
      path: `${parentPath}/newfolder`,
      isFolder: true,
      children: [],
      canMove: false,
      canRename: true,
      isNewFolder: true,
    };

    const parentItem = newItems[parentId];
    if (parentItem) {
      newItems[parentId] = {
        ...parentItem,
        children: [...(parentItem.children || []), tempId],
      };
    }

    itemsRef.current = newItems;
    setItems(newItems);
    newFileIdRef.current = tempId;

    // 确保目标目录展开并聚焦到新项
    setExpandedItems(prev => [...new Set([...prev, parentId])]);
    setFocusedItem(tempId);

    setTimeout(() => {
      treeRef.current?.startRenamingItem?.(tempId);
    }, 100);
  }, []);

  useEffect(() => {
    loadTree();
  }, [loadTree]);

  // WebSocket 连接管理
  useEffect(() => {
    connectWebSocket();

    return () => {
      disconnectWebSocket();
    };
  }, [connectWebSocket, disconnectWebSocket]);

  // 监听快捷键
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'n') {
        if (e.shiftKey) {
          // Ctrl+Shift+N: 新建文件夹
          e.preventDefault();
          createNewFolder();
        } else {
          // Ctrl+N: 新建文件
          e.preventDefault();
          createNewFile();
        }
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [createNewFile]);

  // 懒加载子目录
  const loadChildren = useCallback(async (itemId: string) => {
    const item = itemsRef.current[itemId];
    if (!item || !item.isFolder) return;
    
    try {
      const response = await fetch(`/api/children?path=${encodeURIComponent(item.path)}`);
      const children: FileEntry[] = await response.json();
      
      const newItems = { ...itemsRef.current };
      const childIds: string[] = [];
      
      children.forEach(child => {
        const childId = `${itemId}/${child.name}`;
        childIds.push(childId);
        newItems[childId] = {
          index: childId,
          data: child.name,
          path: child.path,
          isFolder: child.is_directory,
          children: child.is_directory ? [] : undefined,
          canMove: false,
          canRename: !child.is_directory,
        };
      });
      
      newItems[itemId] = { ...item, children: childIds };
      itemsRef.current = newItems;
      setItems(newItems);
    } catch (error) {
      console.error('Failed to load children:', error);
    }
  }, []);

  // 暴露方法给父组件
  useImperativeHandle(ref, () => ({
    createNewFile,
    loadFolder: (path: string) => {
      loadTree(path);
    },
  }));

  const handleDoubleClick = useCallback((item: TreeItem<string>) => {
    if (item.isFolder) return;
    const fileItem = item as FileTreeItem;
    if (fileItem.path && onFileOpen) {
      onFileOpen(fileItem.path, item.data);
    }
  }, [onFileOpen]);

  // 复制到虚拟剪贴板
  const copyToClipboard = useCallback((item: FileTreeItem) => {
    setClipboard({
      path: item.path,
      name: item.data,
      isFolder: !!item.isFolder,
    });
  }, []);

  // 粘贴（复制文件）
  const pasteFromClipboard = useCallback(async (targetItem: FileTreeItem) => {
    if (!clipboard) return;

    const isRoot = String(targetItem.index) === 'root';
    const targetPath = isRoot
      ? `${targetItem.path}/${clipboard.name}`
      : targetItem.isFolder
        ? `${targetItem.path}/${clipboard.name}`
        : `${targetItem.path.substring(0, targetItem.path.lastIndexOf('/'))}/${clipboard.name}`;

    try {
      const response = await fetch('/api/file/copy', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          source_path: clipboard.path,
          target_path: targetPath,
        }),
      });

      const result = await response.json();
      if (result.success) {
        // 刷新当前目录
        if (isRoot) {
          // 根目录需要重新加载整个树
          await loadTree();
        } else {
          const parentId = String(targetItem.index).substring(0, String(targetItem.index).lastIndexOf('/')) || 'root';
          if (targetItem.isFolder) {
            await loadChildren(String(targetItem.index));
          } else if (parentId) {
            await loadChildren(parentId);
          }
        }
      } else {
        console.error('Failed to paste file:', result.message);
      }
    } catch (error) {
      console.error('Error pasting file:', error);
    }
  }, [clipboard, loadTree]);

  // 安全的复制到剪贴板函数
  const safeCopyToClipboard = useCallback(async (text: string) => {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        // 降级方案：使用临时 textarea
        const textarea = document.createElement('textarea');
        textarea.value = text;
        textarea.style.position = 'fixed';
        textarea.style.opacity = '0';
        document.body.appendChild(textarea);
        textarea.select();
        try {
          document.execCommand('copy');
        } catch (err) {
          console.error('复制失败:', err);
        }
        document.body.removeChild(textarea);
      }
    } catch (err) {
      console.error('复制到剪贴板失败:', err);
    }
  }, []);

  // 复制绝对路径到剪贴板
  const copyAbsolutePath = useCallback((item: FileTreeItem) => {
    safeCopyToClipboard(item.path);
  }, [safeCopyToClipboard]);

  // 复制相对路径到剪贴板
  const copyRelativePath = useCallback((item: FileTreeItem) => {
    const rootPath = rootPathRef.current;
    const relativePath = item.path.startsWith(rootPath)
      ? item.path.substring(rootPath.length + 1)
      : item.path;
    safeCopyToClipboard(relativePath);
  }, [safeCopyToClipboard]);

  // 重命名文件
  const renameItem = useCallback((itemId: string) => {
    if (treeRef.current) {
      treeRef.current.startRenamingItem(itemId);
    }
  }, []);

  // 删除文件
  const deleteItem = useCallback(async (item: FileTreeItem) => {
    if (!confirm(`确定要删除 "${item.data}" 吗？`)) return;

    try {
      const response = await fetch(`/api/file?path=${encodeURIComponent(item.path)}`, {
        method: 'DELETE',
      });

      const result = await response.json();
      if (result.success) {
        // 从状态中移除该项
        const newItems = { ...itemsRef.current };
        const itemId = String(item.index);
        delete newItems[itemId];

        // 从父项的 children 中移除
        const parentId = itemId.substring(0, itemId.lastIndexOf('/')) || 'root';
        const parentItem = newItems[parentId];
        if (parentItem && parentItem.children) {
          newItems[parentId] = {
            ...parentItem,
            children: parentItem.children.filter(id => id !== itemId),
          };
        }

        itemsRef.current = newItems;
        setItems(newItems);
      } else {
        console.error('Failed to delete file:', result.message);
      }
    } catch (error) {
      console.error('Error deleting file:', error);
    }
  }, []);

  // 处理右键点击
  const handleContextMenu = useCallback((e: React.MouseEvent, item: FileTreeItem) => {
    e.preventDefault();
    e.stopPropagation();

    setContextMenu({
      visible: true,
      x: e.clientX,
      y: e.clientY,
      itemId: String(item.index),
    });

    // 选中右键点击的项
    setFocusedItem(item.index);
    setSelectedItems([item.index]);
  }, []);

  // 关闭右键菜单
  const closeContextMenu = useCallback(() => {
    setContextMenu(prev => ({ ...prev, visible: false }));
  }, []);

  // 处理空白区域右键点击（粘贴到根目录）
  const handleBackgroundContextMenu = useCallback((e: React.MouseEvent) => {
    // 如果点击的是文件项，不处理
    if ((e.target as HTMLElement).closest('.file-tree-row')) return;

    e.preventDefault();
    e.stopPropagation();

    setContextMenu({
      visible: true,
      x: e.clientX,
      y: e.clientY,
      itemId: 'root',
    });
  }, []);

  // 获取右键菜单项
  const getContextMenuItems = useCallback((item: FileTreeItem): ContextMenuItem[] => {
    const isRoot = String(item.index) === 'root';

    return [
      {
        label: '新建文件',
        action: () => createNewFile(String(item.index)),
      },
      {
        label: '新建文件夹',
        action: () => createNewFolder(String(item.index)),
      },
      { label: '', action: () => {}, divider: true },
      {
        label: '复制',
        action: () => copyToClipboard(item),
        disabled: isRoot,
      },
      {
        label: '粘贴',
        action: () => pasteFromClipboard(item),
        disabled: !clipboard,
      },
      { label: '', action: () => {}, divider: true },
      {
        label: '复制绝对路径',
        action: () => copyAbsolutePath(item),
        disabled: isRoot,
      },
      {
        label: '复制相对路径',
        action: () => copyRelativePath(item),
        disabled: isRoot,
      },
      { label: '', action: () => {}, divider: true },
      {
        label: '重命名',
        action: () => renameItem(String(item.index)),
        disabled: isRoot,
      },
      {
        label: '删除',
        action: () => deleteItem(item),
        disabled: isRoot,
      },
    ];
  }, [clipboard, copyToClipboard, pasteFromClipboard, copyAbsolutePath, copyRelativePath, renameItem, deleteItem, safeCopyToClipboard, createNewFile, createNewFolder]);

  const handleRenameItem = useCallback(async (item: TreeItem<string>, newName: string) => {
    const id = String(item.index);
    const fileItem = item as FileTreeItem;
    const isNewFile = fileItem.isNewFile;
    const isNewFolder = fileItem.isNewFolder;

    if (isNewFile || isNewFolder) {
      // 获取父目录ID
      const parentId = id.substring(0, id.lastIndexOf('/')) || 'root';
      const parentItem = itemsRef.current[parentId];
      const parentPath = parentItem ? parentItem.path : rootPathRef.current;

      // 辅助函数：移除临时项
      const removeTempItem = () => {
        const newItems = { ...itemsRef.current };
        delete newItems[id];
        const parentItem = newItems[parentId];
        if (parentItem && parentItem.children) {
          newItems[parentId] = {
            ...parentItem,
            children: parentItem.children.filter(childId => childId !== id),
          };
        }
        itemsRef.current = newItems;
        setItems(newItems);
        newFileIdRef.current = null;
      };

      if (!newName.trim()) {
        // 如果名称为空，取消创建
        removeTempItem();
        return;
      }

      const newPath = `${parentPath}/${newName}`;

      try {
        if (isNewFolder) {
          // 创建文件夹
          const response = await fetch('/api/folder', {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
            },
            body: JSON.stringify({
              path: newPath,
            }),
          });

          const result = await response.json();
          if (result.success) {
            const newItems = { ...itemsRef.current };
            newItems[id] = {
              ...fileItem,
              data: newName,
              path: newPath,
              isNewFolder: undefined,
            };
            itemsRef.current = newItems;
            setItems(newItems);
            newFileIdRef.current = null;
          } else {
            console.error('Failed to create folder:', result.message);
            removeTempItem();
          }
        } else {
          // 创建文件
          const response = await fetch('/api/file', {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
            },
            body: JSON.stringify({
              path: newPath,
              content: '',
            }),
          });

          const result = await response.json();
          if (result.success) {
            const newItems = { ...itemsRef.current };
            newItems[id] = {
              ...fileItem,
              data: newName,
              path: newPath,
              isNewFile: undefined,
            };
            itemsRef.current = newItems;
            setItems(newItems);
            newFileIdRef.current = null;

            // 自动打开新创建的文件
            onFileOpen?.(newPath, newName);
          } else {
            console.error('Failed to create file:', result.message);
            removeTempItem();
          }
        }
      } catch (error) {
        console.error('Error creating item:', error);
        removeTempItem();
      }
    }
  }, [onFileOpen]);

  const renderItem = useCallback((props: any) => {
    const isFolder = props.item.isFolder;
    const isExpanded = props.context.isExpanded;
    const context = props.context;
    const isRenaming = props.context.isRenaming;
    const fileItem = props.item as FileTreeItem;

    const handleClick = (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      context.focusItem?.();
      context.selectItem?.();
      if (isFolder) {
        context.toggleExpandedState?.();
      }
    };

    const handleDoubleClickEvent = (e: React.MouseEvent) => {
      e.stopPropagation();
      if (!isRenaming) {
        handleDoubleClick(props.item);
      }
    };

    const handleContextMenuEvent = (e: React.MouseEvent) => {
      handleContextMenu(e, fileItem);
    };

    return (
      <div
        {...(context.itemContainerWithChildrenProps || {})}
        className={`file-tree-item ${context.isFocused ? 'focused' : ''} ${context.isSelected ? 'selected' : ''}`}
      >
        <div
          {...(context.itemContainerWithoutChildrenProps || {})}
          className="file-tree-row"
          onClick={handleClick}
          onDoubleClick={handleDoubleClickEvent}
          onContextMenu={handleContextMenuEvent}
        >
          <span className="file-tree-arrow">
            {isFolder && (
              <span className={`arrow-icon ${isExpanded ? 'expanded' : ''}`}>
                ▶
              </span>
            )}
          </span>
          <span className="file-tree-icon">
            {isFolder ? (
              <svg className="folder-icon" viewBox="0 0 24 24" fill="currentColor">
                <path d="M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/>
              </svg>
            ) : (
              <svg className="file-icon" viewBox="0 0 24 24" fill="currentColor">
                <path d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"/>
              </svg>
            )}
          </span>
          <span className="file-tree-label">{props.title}</span>
        </div>
        {props.children}
      </div>
    );
  }, [handleDoubleClick, handleContextMenu]);

  if (loading) {
    return (
      <div className="file-explorer">
        <div className="file-explorer-header">
          <span>文件</span>
          <div className="ws-status" title={wsConnected ? '实时同步已连接' : '实时同步已断开'}>
            <span className={`ws-indicator ${wsConnected ? 'connected' : 'disconnected'}`}></span>
          </div>
          <div className="header-actions">
            <button
              className="header-btn"
              onClick={() => createNewFile()}
              title="新建文件 (Ctrl+N)"
            >
              <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"/>
              </svg>
            </button>
            <button
              className="header-btn"
              onClick={() => createNewFolder()}
              title="新建文件夹 (Ctrl+Shift+N)"
            >
              <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                <path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z"/>
              </svg>
            </button>
            <button
              className="header-btn"
              onClick={() => treeRef.current?.collapseAll?.()}
              title="收起所有目录"
            >
              <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                <path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM3 16h7v-2H3v2z"/>
              </svg>
            </button>
          </div>
        </div>
        <div className="file-explorer-content">
          <div className="loading">加载中...</div>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="file-explorer">
        <div className="file-explorer-header">
          <span>文件</span>
          <div className="ws-status" title={wsConnected ? '实时同步已连接' : '实时同步已断开'}>
            <span className={`ws-indicator ${wsConnected ? 'connected' : 'disconnected'}`}></span>
          </div>
          <div className="header-actions">
            <button
              className="header-btn"
              onClick={() => createNewFile()}
              title="新建文件 (Ctrl+N)"
            >
              <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"/>
              </svg>
            </button>
            <button
              className="header-btn"
              onClick={() => createNewFolder()}
              title="新建文件夹 (Ctrl+Shift+N)"
            >
              <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                <path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z"/>
              </svg>
            </button>
            <button
              className="header-btn"
              onClick={() => treeRef.current?.collapseAll?.()}
              title="收起所有目录"
            >
              <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                <path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM3 16h7v-2H3v2z"/>
              </svg>
            </button>
          </div>
        </div>
        <div className="file-explorer-content">
          <div className="error">错误: {error}</div>
        </div>
      </div>
    );
  }

  const contextMenuItem = contextMenu.itemId ? items[contextMenu.itemId] : null;

  return (
      <div className="file-explorer">
      <div className="file-explorer-header">
        <span>文件</span>
        <div className="ws-status" title={wsConnected ? '实时同步已连接' : '实时同步已断开'}>
          <span className={`ws-indicator ${wsConnected ? 'connected' : 'disconnected'}`}></span>
        </div>
        <div className="header-actions">
          <button
            className="header-btn"
            onClick={() => createNewFile()}
            title="新建文件 (Ctrl+N)"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
              <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"/>
            </svg>
          </button>
          <button
            className="header-btn"
            onClick={() => createNewFolder()}
            title="新建文件夹 (Ctrl+Shift+N)"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
              <path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z"/>
            </svg>
          </button>
          <button
            className="header-btn collapse-btn"
            onClick={() => treeRef.current?.collapseAll?.()}
            title="收起所有目录"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
              <path d="M14 10H3v2h11v-2zm0-4H3v2h11V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM3 16h7v-2H3v2z"/>
            </svg>
          </button>
        </div>
      </div>
      <div className="file-explorer-content" onContextMenu={handleBackgroundContextMenu}>
        <ControlledTreeEnvironment
          items={items}
          getItemTitle={(item) => item.data}
          viewState={{
            'file-tree': {
              focusedItem,
              expandedItems,
              selectedItems,
            },
          }}
          onFocusItem={(item) => setFocusedItem(item.index)}
          onExpandItem={(item) => {
            setExpandedItems(prev => [...new Set([...prev, item.index])]);
            loadChildren(String(item.index));
          }}
          onCollapseItem={(item) => {
            setExpandedItems(prev => prev.filter(id => id !== item.index));
          }}
          onSelectItems={setSelectedItems}
          renderItem={renderItem}
          canRename={true}
          onRenameItem={handleRenameItem}
          keyboardBindings={{
            renameItem: ['f2'],
            abortRenameItem: ['escape'],
          }}
        >
          <Tree ref={treeRef} treeId="file-tree" rootItem="root" treeLabel="文件树" />
        </ControlledTreeEnvironment>
      </div>
      {contextMenu.visible && contextMenuItem && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={getContextMenuItems(contextMenuItem)}
          onClose={closeContextMenu}
        />
      )}
    </div>
  );
});

FileExplorer.displayName = 'FileExplorer';

export default FileExplorer;
