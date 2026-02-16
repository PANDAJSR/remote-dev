# Rust + React 全栈项目

这是一个使用 Rust (Axum) 作为后端，React + Vite 作为前端的全栈应用模板。

## 项目结构

```
.
├── backend/           # Rust 后端
│   ├── src/
│   │   └── main.rs   # 服务器入口
│   ├── static/       # 前端构建输出目录
│   └── Cargo.toml
├── frontend/         # React 前端
│   ├── src/
│   │   ├── App.tsx
│   │   ├── App.css
│   │   ├── main.tsx
│   │   └── index.css
│   ├── index.html
│   ├── package.json
│   ├── tsconfig.json
│   └── vite.config.ts
└── Cargo.toml        # Workspace 配置
```

## 开发环境

### 1. 安装依赖

```bash
# 前端依赖
cd frontend
npm install

# 后端依赖（确保已安装 Rust）
cd ../backend
cargo build
```

### 2. 开发模式

启动后端服务器：
```bash
cd backend
cargo run
```

在另一个终端启动前端开发服务器：
```bash
cd frontend
npm run dev
```

前端开发服务器会在 `http://localhost:5173` 运行，并代理 API 请求到后端。

### 3. 生产构建

构建前端：
```bash
cd frontend
npm run build
```

这会生成静态文件到 `backend/static` 目录。

运行生产服务器：
```bash
cd backend
cargo run --release
```

服务器会在 `http://localhost:3000` 运行，同时提供 API 和前端静态文件。

## API 端点

- `GET /api/health` - 健康检查
- `GET /api/hello` - 测试消息
- `GET /api/users` - 获取用户列表

## 技术栈

- **后端**: Rust + Axum + Tokio
- **前端**: React + TypeScript + Vite
- **构建**: Cargo (Rust) + Vite (前端)