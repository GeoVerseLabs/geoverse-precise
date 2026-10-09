# 安全策略

## 支持的版本

项目处于 0.x 阶段，只为最新发布版本提供修复。

## 报告漏洞

请**不要**通过公开 issue 报告安全问题。

使用 GitHub 的私密安全公告提交：
[Report a vulnerability](https://github.com/GeoVerseLabs/geoverse-precise/security/advisories/new)

请尽量附上受影响的版本、复现步骤与影响范围。我们会在确认后尽快回复，并在修复发布后公开说明。

## 范围说明

geoverse-precise 是纯计算库：除初始化时加载自身的 `.wasm`（浏览器下 fetch、Node 下读文件）外，不发起网络请求、不读写文件。
值得报告的问题主要包括：特定输入导致的 wasm 内存越界或崩溃、超长时间计算（拒绝服务）、
以及 GeoJSON 解析中的资源耗尽。
