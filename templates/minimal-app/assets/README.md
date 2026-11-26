# 随模板分发的字体

模板固定使用这一份字体，不依赖系统字体集合。

| 项目 | 值 |
| --- | --- |
| 字体 | Noto Sans SC Regular |
| 家族名 | `Noto Sans SC` |
| 文件 | `NotoSansSC-Regular.otf`，CFF OpenType，子集 `SC` |
| 来源 | [notofonts/noto-cjk @ f8d1575](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/SubsetOTF/SC/NotoSansSC-Regular.otf) |
| 大小 | 8,331,336 字节 |
| SHA-256 | `faa6c9df652116dde789d351359f3d7e5d2285a2b2a1f04a2d7244df706d5ea9` |
| 版权 | © 2014–2021 Adobe (http://www.adobe.com/) |
| 许可 | SIL Open Font License 1.1，全文见 [`OFL.txt`](OFL.txt) |

字体的 cmap 覆盖 30,890 个码点，包括模板使用的 Latin、数字与 `你好，世界` 等常用简体汉字。发布目录把本文件复制到 exe 旁的 `assets/`；模板从那里读取并以家族名 `Noto Sans SC` 注册。文件名与家族名同时出现在 [模板 README](../README.md) 与 [从空目录开始](../../../docs/SDK/从空目录开始.md#发布构建)。

字体是独立的第三方资源，保留 SIL OFL 1.1；模板代码仍以 `MIT OR Apache-2.0` 发布。
