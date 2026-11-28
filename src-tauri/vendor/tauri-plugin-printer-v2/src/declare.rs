use serde::{Deserialize, Serialize};
 

 

pub struct PrintOptions {
    pub id: String,  // 打印任务 ID
    pub path: String,  // 打印文件路径
    pub printer: String,  // 打印机名称
    pub print_settings: String,  // 打印设置
    pub remove_after_print: Option<bool>,  // 打印完成后删除文件
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PrintHtmlOptions {
    pub id: String,  // 打印任务 ID
    pub html: String,  // HTML 内容
    pub printer: String,  // 打印机名称
    pub print_settings: String,  // 打印设置
    pub remove_after_print: Option<bool>,  // 打印完成后删除文件
    pub page_size: Option<String>,  // A4, Letter 等
    pub orientation: Option<String>,  // portrait, landscape
    pub margin: Option<PrintMargin>,  // 页边距
    pub quality: Option<u32>,  // 打印质量
    pub grayscale: Option<bool>,  // 灰度打印
    pub copies: Option<u32>,  // 打印份数
    pub page_width: Option<f64>,  // 自定义页面宽度 (mm)
    pub page_height: Option<f64>,  // 自定义页面高度 (mm)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrintMargin {
    pub top: Option<f64>,  // 上边距
    pub right: Option<f64>,  // 右边距
    pub bottom: Option<f64>,  // 下边距
    pub left: Option<f64>,  // 左边距
    pub unit: Option<String>,  // mm, cm, inch
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrintPdfUrlOptions {
    pub id: String,  // 打印任务 ID
    pub url: String,  // PDF文件URL
    pub printer: String,  // 打印机名称
    pub print_settings: String,  // 打印设置
    pub remove_after_print: Option<bool>,  // 打印完成后删除文件
    pub timeout_seconds: Option<u64>,  // 下载超时时间（秒）
    pub temp_dir: Option<String>,  // 临时文件目录
}