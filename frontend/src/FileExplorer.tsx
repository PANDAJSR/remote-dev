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
    } catch (err: any) {
      console.error('Error fetching directory tree:', err);
      setError(err.message);
      setLoading(false);
    }
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
