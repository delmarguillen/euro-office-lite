<div align="center">

# Tauri Plugin Printer V2

[![Crates.io](https://img.shields.io/crates/v/tauri-plugin-printer-v2.svg)](https://crates.io/crates/tauri-plugin-printer-v2)
[![NPM](https://img.shields.io/npm/v/tauri-plugin-printer-api.svg)](https://www.npmjs.com/package/tauri-plugin-printer-api)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-v2.0-orange.svg)](https://tauri.app/)

**一个功能强大的 Tauri V2 打印机插件**

支持 PDF/HTML 打印、打印机管理、任务控制等完整功能

[安装](#-安装) • [使用](#-使用方法) • [API文档](#-api-文档) • [示例](#-示例)

</div>

---

## 🚀 快速开始

### 5分钟上手指南

1. **安装插件**
   ```bash
   cargo add tauri-plugin-printer-v2
   npm i tauri-plugin-printer-v2
   ```

2. **注册插件**
   ```rust
   // src-tauri/src/lib.rs
   use tauri_plugin_printer_v2::init;
   
   #[cfg_attr(mobile, tauri::mobile_entry_point)]
   pub fn run() {
       tauri::Builder::default()
           .plugin(init())
           .run(tauri::generate_context!())
           .expect("error while running tauri application");
   }
   ```

3. **配置权限**
   ```json
   // src-tauri/capabilities/default.json
   {
     "permissions": [
       "printer:allow-get-printers",
       "printer:allow-print-pdf",
       "printer:allow-print-html"
     ]
   }
   ```

4. **开始使用**
   ```javascript
   import { getPrinters, printPdf } from 'tauri-plugin-printer-v2';
   
   // 获取打印机
   const printers = JSON.parse(await getPrinters());
   console.log('可用打印机:', printers);
   
   // 打印PDF
   await printPdf({
     path: '/path/to/document.pdf',
     printer: printers[0].name
   });
   ```

🎉 **恭喜！** 您已成功集成打印机插件，现在可以开始打印文档了。

## 致谢

本项目基于以下开源项目开发：
- [alfianlensundev/tauri-plugin-printer](https://github.com/alfianlensundev/tauri-plugin-printer) - 原始 Tauri 打印机插件
- [adao99/tauri-plugin-printer-v2](https://github.com/adao99/tauri-plugin-printer-v2) - Tauri v2.0 版本的打印机插件

感谢原作者们的贡献和开源精神。

## ✨ 特性

- 🖨️ 获取系统打印机列表
- 📄 打印 PDF 文件
- 🌐 打印 HTML 内容
- 📋 管理打印任务（暂停、恢复、重启、删除）
- 🔍 按名称查询打印机
- 📊 获取打印任务状态
- 🌐 支持中文打印机名称
- 🔧 完全兼容 Tauri V2 稳定版

## 📦 安装

### 方法一：使用 crates.io（推荐）

```bash
# 添加 Rust 依赖
cargo add tauri-plugin-printer-v2

# 安装前端 API
npm i tauri-plugin-printer-v2
```

### 方法二：使用 Tauri CLI

```bash
npx tauri add https://github.com/chen-collab/tauri-plugin-printer
```

### 方法三：手动安装

1. 在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
tauri-plugin-printer-v2 = "0.2.0"
# 或使用 Git 版本
# tauri-plugin-printer-v2 = { git = "https://github.com/chen-collab/tauri-plugin-printer", branch = "chen" }
```

2. 在 `src-tauri/src/lib.rs` 中注册插件：

```rust
use tauri_plugin_printer_v2::init;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

3. 在 `src-tauri/capabilities/default.json` 中添加权限：

```json
{
  "permissions": [
    "printer:default"
  ]
}
```

4. 安装前端依赖：

```bash
npm i tauri-plugin-printer-v2
```

## 🚀 使用方法

### 基础示例

```javascript
import { ping, getPrinters } from 'tauri-plugin-printer-v2';

// 测试插件连接
const response = await ping({ value: 'Hello from frontend!' });
console.log(response);

// 获取打印机列表
const printers = await getPrinters();
console.log('可用打印机:', printers);
```

### 完整 API 示例

```javascript
import {
  ping,
  getPrinters,
  getPrintersByName,
  printPdf,
  printHtml,
  getJobs,
  getJobsById,
  resumeJob,
  restartJob,
  pauseJob,
  removeJob
} from 'tauri-plugin-printer-v2';

// 1. 获取所有打印机
const allPrinters = await getPrinters();

// 2. 按名称获取特定打印机
const specificPrinter = await getPrintersByName('Microsoft Print to PDF');

// 3. 打印 PDF 文件
const printResult = await printPdf({
  path: '/path/to/your/file.pdf',
  printer: 'Microsoft Print to PDF',
  pages: '1-3',
  subset: 'odd'
});

// 4. 打印 HTML 内容
const htmlPrintResult = await printHtml({
  html: '<h1>Hello World</h1><p>这是一个HTML打印测试</p>',
  printer: 'Microsoft Print to PDF'
});

// 4.1 使用自定义纸张大小打印 HTML
const customSizeHtmlResult = await printHtml({
  html: '<h1>自定义尺寸</h1><p>使用 200mm x 300mm 的自定义纸张</p>',
  printer: 'Microsoft Print to PDF',
  page_width: 200,  // 宽度 200mm
  page_height: 300, // 高度 300mm
  orientation: 'portrait'
});

// 5. 获取打印任务
const jobs = await getJobs('Microsoft Print to PDF');

// 6. 管理打印任务
const jobId = '123';
const printerName = 'Microsoft Print to PDF';

// 暂停任务
await pauseJob(printerName, jobId);

// 恢复任务
await resumeJob(printerName, jobId);

// 重启任务
await restartJob(printerName, jobId);

// 删除任务
await removeJob(printerName, jobId);
```

## 📋 示例

### 实际应用场景

#### 1. 打印发票或报告

```javascript
import { printHtml, getPrinters } from 'tauri-plugin-printer-v2';

// 生成发票HTML
const invoiceHtml = `
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>发票</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; }
        .header { text-align: center; margin-bottom: 30px; }
        .invoice-details { margin-bottom: 20px; }
        table { width: 100%; border-collapse: collapse; }
        th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
        th { background-color: #f2f2f2; }
    </style>
</head>
<body>
    <div class="header">
        <h1>发票</h1>
        <p>发票号码: INV-2024-001</p>
    </div>
    <div class="invoice-details">
        <p><strong>客户:</strong> 张三</p>
        <p><strong>日期:</strong> 2024-01-15</p>
    </div>
    <table>
        <tr><th>项目</th><th>数量</th><th>单价</th><th>总计</th></tr>
        <tr><td>产品A</td><td>2</td><td>¥100</td><td>¥200</td></tr>
        <tr><td>产品B</td><td>1</td><td>¥150</td><td>¥150</td></tr>
    </table>
    <p style="text-align: right; margin-top: 20px;"><strong>总计: ¥350</strong></p>
</body>
</html>
`;

// 打印发票
try {
    const result = await printHtml({
        html: invoiceHtml,
        printer: 'Microsoft Print to PDF' // 或选择其他打印机
    });
    console.log('发票打印成功:', result);
} catch (error) {
    console.error('打印失败:', error);
}
```

#### 2. 批量打印PDF文件

```javascript
import { printPdf, getPrinters } from 'tauri-plugin-printer-v2';

const pdfFiles = [
    '/path/to/document1.pdf',
    '/path/to/document2.pdf',
    '/path/to/document3.pdf'
];

// 获取默认打印机
const printers = JSON.parse(await getPrinters());
const defaultPrinter = printers.find(p => p.is_default)?.name || printers[0]?.name;

// 批量打印
for (const filePath of pdfFiles) {
    try {
        await printPdf({
            path: filePath,
            printer: defaultPrinter,
            pages: '1-10' // 只打印前10页
        });
        console.log(`已打印: ${filePath}`);
    } catch (error) {
        console.error(`打印失败 ${filePath}:`, error);
    }
}
```

#### 3. 打印机状态监控

```javascript
import { getPrinters, getJobs } from 'tauri-plugin-printer-v2';

// 监控打印机状态
async function monitorPrinters() {
    try {
        const printers = JSON.parse(await getPrinters());
        
        for (const printer of printers) {
            console.log(`打印机: ${printer.name}`);
            console.log(`状态: ${printer.status}`);
            console.log(`是否默认: ${printer.is_default ? '是' : '否'}`);
            
            // 获取打印任务
            const jobs = JSON.parse(await getJobs(printer.name));
            console.log(`待处理任务: ${jobs.length}`);
            
            console.log('---');
        }
    } catch (error) {
        console.error('获取打印机信息失败:', error);
    }
}

// 每30秒检查一次
setInterval(monitorPrinters, 30000);
```

## 📚 API 文档

### `ping(request: PingRequest): Promise<PingResponse>`
测试插件连接状态。

### `getPrinters(): Promise<string>`
获取系统中所有可用的打印机列表，返回 JSON 格式的字符串。

### `getPrintersByName(name: string): Promise<string>`
根据打印机名称获取特定打印机信息。

### `printPdf(options: PrintOptions): Promise<string>`
打印 PDF 文件。

**PrintOptions 参数：**
- `path`: PDF 文件路径
- `printer`: 打印机名称
- `pages`: 页面范围（可选）
- `subset`: 页面子集（可选）

### `printHtml(options: HtmlPrintOptions): Promise<string>`
打印 HTML 内容，支持自定义纸张大小。

**HtmlPrintOptions 参数：**
- `html`: HTML 内容字符串
- `printer`: 打印机名称
- `page_width`: 自定义页面宽度（毫米，可选）
- `page_height`: 自定义页面高度（毫米，可选）
- `page_size`: 预定义纸张大小（如 A4、A3、Letter 等，可选）
- `orientation`: 页面方向（portrait/landscape，可选）
- `margin`: 页边距设置（可选）
- `quality`: 打印质量（可选）
- `grayscale`: 是否灰度打印（可选）
- `copies`: 打印份数（可选）

### 打印任务管理

- `getJobs(printer: string): Promise<string>` - 获取打印机的所有任务
- `getJobsById(printer: string, jobId: string): Promise<string>` - 获取特定任务信息
- `pauseJob(printer: string, jobId: string): Promise<string>` - 暂停打印任务
- `resumeJob(printer: string, jobId: string): Promise<string>` - 恢复打印任务
- `restartJob(printer: string, jobId: string): Promise<string>` - 重启打印任务
- `removeJob(printer: string, jobId: string): Promise<string>` - 删除打印任务

## 🛠️ 开发

### 运行示例应用

```bash
# 克隆仓库
git clone https://github.com/chen-collab/tauri-plugin-printer.git
cd tauri-plugin-printer

# 构建插件
npm run build

# 运行示例应用
cd examples/tauri-app
npm install
npm run tauri:dev
```

### 项目结构

```
tauri-plugin-printer/
├── src/                    # Rust 源代码
│   ├── lib.rs             # 插件主入口
│   ├── commands.rs        # Tauri 命令定义
│   ├── desktop.rs         # 桌面端实现
│   ├── windows.rs         # Windows 特定实现
│   └── ...
├── guest-js/              # JavaScript API
│   └── index.ts           # 前端 API 定义
├── permissions/           # 权限配置
├── examples/              # 示例应用
│   └── tauri-app/         # Vue.js 示例
└── dist-js/               # 构建后的 JS 文件
```

## 🔧 权限配置

插件使用以下权限：

```toml
[default]
description = "Default permissions for the plugin"
permissions = [
  "allow-ping",
  "allow-create-temp-file", 
  "allow-remove-temp-file", 
  "allow-get-printers", 
  "allow-get-printers-by-name", 
  "allow-print-pdf", 
  "allow-print-html",
  "allow-get-jobs", 
  "allow-get-jobs-by-id", 
  "allow-resume-job", 
  "allow-restart-job", 
  "allow-pause-job", 
  "allow-remove-job"
]
```

## 🐛 已知问题

- 目前主要支持 Windows 系统
- 某些打印机驱动可能不完全兼容
- 大文件打印可能需要额外的内存管理
- HTML 打印在某些复杂布局下可能出现格式问题

## 🔧 故障排除

### 常见问题

#### Q: 无法获取打印机列表
**A:** 请检查以下几点：
- 确保系统中已安装打印机
- 检查打印机服务是否正常运行
- 确认应用具有访问打印机的权限

#### Q: PDF 打印失败
**A:** 可能的解决方案：
- 确保 PDF 文件路径正确且文件存在
- 检查 PDF 文件是否损坏
- 确认系统已安装 PDF 阅读器（如 Adobe Reader）
- 尝试使用绝对路径而非相对路径

#### Q: HTML 打印格式不正确
**A:** 建议：
- 使用简单的 CSS 样式，避免复杂的布局
- 设置合适的页面大小和边距
- 测试时先打印到 PDF 查看效果
- 避免使用 JavaScript 和外部资源

#### Q: 权限配置问题
**A:** 确保在 `src-tauri/capabilities/default.json` 中正确配置了权限：
```json
{
  "permissions": [
    "printer:allow-ping",
    "printer:allow-get-printers",
    "printer:allow-get-printers-by-name",
    "printer:allow-print-pdf",
    "printer:allow-print-html",
    "printer:allow-get-jobs",
    "printer:allow-restart-job",
    "printer:allow-pause-job",
    "printer:allow-resume-job",
    "printer:allow-remove-job"
  ]
}
```

### 调试技巧

1. **启用详细日志**：
   ```bash
   RUST_LOG=debug npm run tauri dev
   ```

2. **检查打印机状态**：
   ```javascript
   const printers = JSON.parse(await getPrinters());
   console.log('可用打印机:', printers);
   ```

3. **测试连接**：
   ```javascript
   try {
     const result = await ping();
     console.log('插件连接正常:', result);
   } catch (error) {
     console.error('插件连接失败:', error);
   }
   ```

## ⚡ 性能优化

### 最佳实践

#### 1. 打印大文件时的优化

```javascript
// 对于大型PDF文件，建议分页打印
const largePdfPath = '/path/to/large-document.pdf';

// 分批打印，每次10页
for (let startPage = 1; startPage <= totalPages; startPage += 10) {
    const endPage = Math.min(startPage + 9, totalPages);
    
    await printPdf({
        path: largePdfPath,
        printer: printerName,
        pages: `${startPage}-${endPage}`
    });
    
    // 添加延迟避免打印队列过载
    await new Promise(resolve => setTimeout(resolve, 1000));
}
```

#### 2. HTML 打印优化

```javascript
// 优化的HTML结构
const optimizedHtml = `
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <style>
        /* 打印专用样式 */
        @media print {
            body { margin: 0; padding: 20px; }
            .no-print { display: none; }
            .page-break { page-break-before: always; }
        }
        
        /* 基础样式 */
        body {
            font-family: 'Arial', sans-serif;
            font-size: 12pt;
            line-height: 1.4;
        }
        
        table {
            width: 100%;
            border-collapse: collapse;
            margin-bottom: 20px;
        }
        
        th, td {
            border: 1px solid #000;
            padding: 8px;
            text-align: left;
        }
    </style>
</head>
<body>
    <!-- 内容 -->
</body>
</html>
`;
```

#### 3. 错误处理和重试机制

```javascript
// 带重试的打印函数
async function printWithRetry(printFunction, options, maxRetries = 3) {
    for (let attempt = 1; attempt <= maxRetries; attempt++) {
        try {
            const result = await printFunction(options);
            return result;
        } catch (error) {
            console.warn(`打印尝试 ${attempt} 失败:`, error.message);
            
            if (attempt === maxRetries) {
                throw new Error(`打印失败，已重试 ${maxRetries} 次: ${error.message}`);
            }
            
            // 指数退避延迟
            const delay = Math.pow(2, attempt) * 1000;
            await new Promise(resolve => setTimeout(resolve, delay));
        }
    }
}

// 使用示例
try {
    await printWithRetry(printPdf, {
        path: '/path/to/document.pdf',
        printer: 'HP LaserJet'
    });
} catch (error) {
    console.error('最终打印失败:', error);
}
```

#### 4. 打印队列管理

```javascript
class PrintQueue {
    constructor(maxConcurrent = 2) {
        this.queue = [];
        this.running = [];
        this.maxConcurrent = maxConcurrent;
    }
    
    async add(printTask) {
        return new Promise((resolve, reject) => {
            this.queue.push({ task: printTask, resolve, reject });
            this.process();
        });
    }
    
    async process() {
        if (this.running.length >= this.maxConcurrent || this.queue.length === 0) {
            return;
        }
        
        const { task, resolve, reject } = this.queue.shift();
        const promise = task().then(resolve).catch(reject);
        
        this.running.push(promise);
        
        promise.finally(() => {
            this.running = this.running.filter(p => p !== promise);
            this.process();
        });
    }
}

// 使用打印队列
const printQueue = new PrintQueue(2); // 最多同时2个打印任务

// 添加打印任务
const files = ['file1.pdf', 'file2.pdf', 'file3.pdf'];
for (const file of files) {
    printQueue.add(() => printPdf({ path: file, printer: 'Default' }));
}
```

### 内存管理

- **避免在内存中保存大量HTML内容**
- **及时清理打印任务引用**
- **使用流式处理大文件**
- **定期检查和清理打印队列**

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

## 📄 许可证

MIT License

## 🙏 致谢

本项目基于以下仓库开发：

- [Alfian Lensun's Original Plugin Repository](https://github.com/alfianlensundev/tauri-plugin-printer)
- [adao99's Fork for Tauri V2 Beta](https://github.com/adao99/tauri-plugin-printer-v2)

感谢原作者的贡献！

## 📝 更新日志

### v0.2.0 (最新)
- ✨ **新增 HTML 打印功能** - 支持直接打印 HTML 内容，包括自定义纸张大小（page_width/page_height）
- 🔧 **改进错误处理** - 更详细的错误信息和调试支持
- 📚 **完善文档** - 添加详细的使用示例和故障排除指南
- ⚡ **性能优化** - 改进大文件打印和内存管理
- 🛡️ **增强权限控制** - 新增 `allow-print-html` 权限
- 🐛 **修复已知问题** - 解决打印机状态获取和任务管理的稳定性问题

### v0.1.1
- 🐛 修复打印机列表获取问题
- 📖 改进 API 文档
- 🔧 优化错误处理机制

### v0.1.0
- 🎉 初始版本发布
- ✅ 兼容 Tauri V2 稳定版
- ✅ 修复中文打印机名称乱码问题
- ✅ 添加 ping 命令
- ✅ 完善权限配置
- ✅ 提供完整的示例应用