import { useState, useEffect, useCallback, useRef } from 'react';
import Editor from '@monaco-editor/react';
import './EditorPanel.css';

interface EditorPanelProps {
  filePath: string;
  fileName: string;
  onEditorMount?: (api: any) => void;
  onEditorUnmount?: () => void;
  initialContent?: string;
}

export default function EditorPanel({ filePath, fileName, onEditorMount, onEditorUnmount, initialContent }: EditorPanelProps) {
  const [content, setContent] = useState<string>(initialContent || '');
  const [originalContent, setOriginalContent] = useState<string>(initialContent || '');
  const [loading, setLoading] = useState(!initialContent);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [hasChanges, setHasChanges] = useState(false);
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);

  useEffect(() => {
    if (initialContent !== undefined) {
      // 新建文件，不需要从服务器加载
      return;
    }
    
    fetch(`/api/file?path=${encodeURIComponent(filePath)}`)
      .then(res => res.json())
      .then(data => {
        if (data.error) {
          setError(data.error);
        } else {
          setContent(data.content);
          setOriginalContent(data.content);
          setHasChanges(false);
        }
        setLoading(false);
      })
      .catch(err => {
        setError(err.message);
        setLoading(false);
      });
  }, [filePath, initialContent]);

  const handleEditorChange = useCallback((value: string | undefined) => {
    if (value !== undefined) {
      setContent(value);
      setHasChanges(value !== originalContent);
    }
  }, [originalContent]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      const response = await fetch('/api/file', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          path: filePath,
          content: content,
        }),
      });

      const result = await response.json();
      if (result.success) {
        setOriginalContent(content);
        setHasChanges(false);
      } else {
        setError(result.message);
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setSaving(false);
    }
  }, [filePath, content]);

  const handleEditorDidMount = useCallback((editor: any, monaco: any) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    
    // 添加 Ctrl+S 保存快捷键
    editor.addCommand(
      monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
      () => {
        handleSave();
      }
    );
    
    // 报告编辑器 API 给父组件
    if (onEditorMount) {
      onEditorMount(editor);
    }
  }, [onEditorMount, handleSave]);

  // 组件卸载时清理
  useEffect(() => {
    return () => {
      if (onEditorUnmount) {
        onEditorUnmount();
      }
    };
  }, [onEditorUnmount]);

  const getLanguage = (filename: string): string => {
    const ext = filename.split('.').pop()?.toLowerCase();
    const languageMap: { [key: string]: string } = {
      'js': 'javascript',
      'jsx': 'javascript',
      'ts': 'typescript',
      'tsx': 'typescript',
      'json': 'json',
      'html': 'html',
      'css': 'css',
      'scss': 'scss',
      'less': 'less',
      'py': 'python',
      'rs': 'rust',
      'go': 'go',
      'java': 'java',
      'c': 'c',
      'cpp': 'cpp',
      'h': 'c',
      'hpp': 'cpp',
      'md': 'markdown',
      'yaml': 'yaml',
      'yml': 'yaml',
      'xml': 'xml',
      'sh': 'shell',
      'bash': 'shell',
      'ps1': 'powershell',
      'sql': 'sql',
    };
    return languageMap[ext || ''] || 'plaintext';
  };

  if (loading) {
    return (
      <div className="editor-panel">
        <div className="editor-loading">加载中...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="editor-panel">
        <div className="editor-error">错误: {error}</div>
      </div>
    );
  }

  return (
    <div className="editor-panel">
      <div className="editor-toolbar">
        <div className="editor-file-info">
          <span className="editor-filename">{fileName}</span>
          {hasChanges && <span className="editor-unsaved">●</span>}
        </div>
        <button 
          className="editor-save-btn"
          onClick={handleSave}
          disabled={saving || !hasChanges}
        >
          {saving ? '保存中...' : '保存 (Ctrl+S)'}
        </button>
      </div>
      <div className="editor-container">
        <Editor
          height="100%"
          language={getLanguage(fileName)}
          value={content}
          onChange={handleEditorChange}
          onMount={handleEditorDidMount}
          theme="vs-dark"
          options={{
            minimap: { enabled: true },
            fontSize: 14,
            lineNumbers: 'on',
            roundedSelection: false,
            scrollBeyondLastLine: false,
            readOnly: false,
            automaticLayout: true,
            wordWrap: 'on',
          }}
        />
      </div>
    </div>
  );
}
