import { useState, useRef, useEffect, useCallback } from 'react';
import './MenuBar.css';

interface MenuItem {
  label: string;
  action?: () => void;
  shortcut?: string;
  isCheckbox?: boolean;
  checked?: boolean;
  disabled?: boolean;
  divider?: boolean;
}

interface MenuDefinition {
  label: string;
  items: MenuItem[];
}

// 编辑器 API 接口
interface EditorApi {
  trigger: (source: string, action: string) => void;
  getModel: () => { getValue: () => string } | null;
  setSelection: (range: any) => void;
  executeEdits: (source: string, edits: any[]) => void;
  getSelection: () => any;
  focus: () => void;
}

interface MenuBarProps {
  onNewFile?: () => void;
  onSave?: () => void;
  onOpenFolder?: (path: string) => void;
  activeEditor?: EditorApi | null;
}

export default function MenuBar({ onNewFile, onSave, onOpenFolder, activeEditor }: MenuBarProps) {
  const [activeMenu, setActiveMenu] = useState<string | null>(null);
  const [autoSave, setAutoSave] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // 点击外部关闭菜单
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setActiveMenu(null);
      }
    };

    if (activeMenu) {
      document.addEventListener('mousedown', handleClickOutside);
      return () => document.removeEventListener('mousedown', handleClickOutside);
    }
  }, [activeMenu]);

  const handleMenuClick = useCallback((menuLabel: string) => {
    setActiveMenu(activeMenu === menuLabel ? null : menuLabel);
  }, [activeMenu]);

  const handleMenuItemClick = useCallback((item: MenuItem) => {
    if (item.disabled) return;
    
    if (item.isCheckbox) {
      // 对于复选框，切换状态但不关闭菜单
      if (item.label === '自动保存') {
        setAutoSave(!autoSave);
      }
    } else {
      // 执行动作并关闭菜单
      item.action?.();
      setActiveMenu(null);
    }
  }, [autoSave]);

  // 编辑操作函数
  const handleUndo = useCallback(() => {
    activeEditor?.trigger('menu', 'undo');
  }, [activeEditor]);

  const handleRedo = useCallback(() => {
    activeEditor?.trigger('menu', 'redo');
  }, [activeEditor]);

  const handleCopy = useCallback(() => {
    activeEditor?.trigger('menu', 'editor.action.clipboardCopyAction');
  }, [activeEditor]);

  const handlePaste = useCallback(() => {
    activeEditor?.trigger('menu', 'editor.action.clipboardPasteAction');
  }, [activeEditor]);

  const handleOpenFolder = useCallback(() => {
    const path = window.prompt('请输入文件夹路径:', '');
    if (path && path.trim()) {
      onOpenFolder?.(path.trim());
    }
  }, [onOpenFolder]);

  const menus: MenuDefinition[] = [
    {
      label: '文件',
      items: [
        { label: '打开文件夹', action: handleOpenFolder },
        { divider: true } as MenuItem,
        { label: '新建', action: onNewFile, shortcut: 'Ctrl+N' },
        { label: '保存', action: onSave, shortcut: 'Ctrl+S' },
        { divider: true } as MenuItem,
        { 
          label: '自动保存', 
          isCheckbox: true, 
          checked: autoSave,
          action: () => setAutoSave(!autoSave)
        },
      ],
    },
    {
      label: '编辑',
      items: [
        { label: '撤销', action: handleUndo, shortcut: 'Ctrl+Z', disabled: !activeEditor },
        { label: '恢复', action: handleRedo, shortcut: 'Ctrl+Y', disabled: !activeEditor },
        { divider: true } as MenuItem,
        { label: '复制', action: handleCopy, shortcut: 'Ctrl+C', disabled: !activeEditor },
        { label: '粘贴', action: handlePaste, shortcut: 'Ctrl+V', disabled: !activeEditor },
      ],
    },
  ];

  return (
    <div className="menubar" ref={menuRef}>
      <div className="menubar-brand">
        <span className="menubar-brand-text">Editor</span>
      </div>
      <div className="menubar-menus">
        {menus.map((menu) => (
          <div
            key={menu.label}
            className={`menubar-menu ${activeMenu === menu.label ? 'active' : ''}`}
          >
            <button
              className="menubar-menu-button"
              onClick={() => handleMenuClick(menu.label)}
              onMouseEnter={() => activeMenu && setActiveMenu(menu.label)}
            >
              {menu.label}
            </button>
            {activeMenu === menu.label && (
              <div className="menubar-dropdown">
                {menu.items.map((item, index) => (
                  item.divider ? (
                    <div key={index} className="menubar-divider" />
                  ) : (
                    <button
                      key={index}
                      className={`menubar-dropdown-item ${item.disabled ? 'disabled' : ''}`}
                      onClick={() => handleMenuItemClick(item)}
                    >
                      <span className="menubar-item-label">
                        {item.isCheckbox && (
                          <span className={`menubar-checkbox ${item.checked ? 'checked' : ''}`}>
                            {item.checked && '✓'}
                          </span>
                        )}
                        {item.label}
                      </span>
                      {item.shortcut && (
                        <span className="menubar-item-shortcut">{item.shortcut}</span>
                      )}
                    </button>
                  )
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
