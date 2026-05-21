# ipssh

[English](README.md)

`ipssh` 是 image paste ssh 的缩写。它是一个 Windows OpenSSH 包装器，用来在 SSH 会话中粘贴剪贴板图片。

当你远程连接到 SSH 服务器，并在服务器上使用 Claude Code、Codex 等终端型 AI 编程工具时，通常无法直接粘贴截图或其他图片。一些 SSH 客户端对多行文本粘贴的处理也不可靠，可能把多行文本当成多条 shell 命令执行。

`ipssh` 只解决图片粘贴问题，同时保留普通终端行为。它启动系统里的 `ssh.exe`，让 Windows Terminal 和 OpenSSH 继续负责输入输出；额外增加一个功能：当按下配置的图片粘贴快捷键，并且 Windows 剪贴板中是图片时，`ipssh` 使用 `scp.exe` 上传图片，并把远端图片路径粘贴到当前 SSH 会话中。

`ipssh` 不实现 SSH 协议。它调用 Windows 上已经安装的 `ssh.exe` 和 `scp.exe` 命令行工具。这些工具通常来自 Windows OpenSSH 可选功能，或 Windows Git 安装包。

设计细节见 [docs/design.zh-CN.md](docs/design.zh-CN.md)。英文版见 [README.md](README.md) 和 [docs/design.md](docs/design.md)。

## 工作原理

`ipssh` 不实现自己的终端模拟器。SSH 子进程继承当前控制台，所以普通输入、`Shift+Insert`、多行粘贴、`Ctrl+C`、`Ctrl+D`、颜色和全屏终端程序都尽量保持与直接使用 OpenSSH 一致。

同时，后台工作线程会安装一个低级 Windows 键盘钩子来监听配置的快捷键。按下快捷键时，它检查剪贴板。如果剪贴板中是图片，就通过 OpenSSH 工具上传图片，并临时把生成的远端路径放入剪贴板，再模拟 `Shift+Insert` 把路径粘贴进 SSH 会话。如果同一个 `ipssh` 会话中再次粘贴同一张剪贴板图片，会复用上一次的远端路径，不重复上传。如果剪贴板不是图片，`ipssh` 不接管，交给终端原样处理。

![ipssh 架构](docs/assets/architecture.zh-CN.png)

## 环境要求

- Windows 10 或 Windows 11。
- `PATH` 中可找到 OpenSSH 客户端工具：`ssh.exe` 和 `scp.exe`。它们通常随 Windows Git 安装。
- 可连接的 SSH 目标，例如 `user@host`，或 `%USERPROFILE%\.ssh\config` 中的主机别名。

在 PowerShell 中检查：

```powershell
ssh -V
scp -V
```

`scp -V` 可能输出用法而不是版本号，只要命令存在即可。

## 安装

运行安装包：

```powershell
.\ipssh-0.1.2-windows-x64-installer.exe
```

默认安装路径：

```text
Binary: %LOCALAPPDATA%\Programs\ipssh\ipssh.exe
Config: %APPDATA%\ipssh\config.toml
```

安装器只会在配置文件不存在时创建默认配置，也会把程序目录加入当前用户的 `PATH`。安装后请打开新的 PowerShell 或 Windows Terminal 窗口，然后验证：

```powershell
ipssh --help
```

不修改 `PATH`：

```powershell
.\ipssh-0.1.2-windows-x64-installer.exe --no-path
```

卸载：

```powershell
.\ipssh-0.1.2-windows-x64-installer.exe --uninstall
```

卸载会移除程序和 PATH 项，但保留用户配置文件。

## 基本用法

像 `ssh` 一样连接：

```powershell
ipssh user@example.com
```

使用 SSH 配置中的主机别名：

```powershell
ipssh my-server
```

需要传递 OpenSSH 参数时，把它们放在 `--` 后：

```powershell
ipssh -- user@example.com -p 2222 -i C:\Users\me\.ssh\id_ed25519
```

常见参数如 `-p`、`-i`、`-F`、`-J`、`-o` 会复用于上传子进程。`ipssh` 会自动把 `ssh -p` 转换为 `scp -P`。

## Windows Terminal 配置

建议搭配 Windows Terminal 使用。可以创建一个专用 Windows Terminal 配置，命令行中启动 PowerShell 并运行 `ipssh`：

```text
%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe "ipssh james@192.168.100.202"
```

把 `james@192.168.100.202` 替换成你的 SSH 目标或主机别名。

## 粘贴图片

1. 把图片复制到 Windows 剪贴板。
2. 聚焦 `ipssh` 终端。
3. 按图片粘贴快捷键，默认是 `Alt+V`。

`ipssh` 会：

1. 把剪贴板图片保存为临时 PNG。
2. 运行 `ssh` 创建远端上传目录。
3. 运行 `scp` 上传图片。
4. 把渲染后的远端路径粘贴到当前 SSH 会话。

如果剪贴板图片与本次 `ipssh` 会话中上一次上传的图片相同，`ipssh` 会跳过 `ssh` 和 `scp`，直接粘贴缓存的远端路径。

默认远端目录：

```text
/tmp/ipssh-images
```

默认插入文本是远端文件路径：

```text
/tmp/ipssh-images/20260521-153012-a1b2c3.png
```

## 文本粘贴

文本粘贴由 Windows Terminal 和 OpenSSH 处理，不由 `ipssh` 处理。

这是有意设计。`Shift+Insert`、多行粘贴、shell bracketed paste 行为和普通终端编辑应当与直接使用 `ssh.exe` 一致。当配置的快捷键按下时，如果剪贴板是文本或空内容，`ipssh` 不上传，也不合成文本输入。

## 配置

默认配置文件：

```text
%APPDATA%\ipssh\config.toml
```

示例：

```toml
paste_hotkey = "alt+v"
remote_dir = "/tmp/ipssh-images"
image_format = "png"
non_image_paste = "text"
template = "{remote_path}"

[upload]
filename_pattern = "{timestamp}-{random}.{ext}"
```

配置项：

- `paste_hotkey`：图片粘贴快捷键，例如 `alt+v`、`ctrl+shift+v`。
- `remote_dir`：远端图片上传目录。
- `image_format`：图片输出格式，目前支持 `png`。
- `non_image_paste`：保留字段；当前非图片粘贴交给终端处理。
- `template`：上传后粘贴的文本模板。
- `filename_pattern`：远端文件名模板。

模板变量：

- `{remote_path}`：完整远端路径。
- `{remote_dir}`：远端上传目录。
- `{filename}`：生成的文件名。

文件名变量：

- `{timestamp}`：本地时间戳，格式为 `YYYYMMDD-HHMMSS`。
- `{random}`：六位随机后缀。
- `{ext}`：文件扩展名，目前为 `png`。

## 命令行覆盖

命令行参数会覆盖配置文件。

自定义远端目录：

```powershell
ipssh --remote-dir "~/uploads" -- user@example.com
```

插入 Markdown 图片语法：

```powershell
ipssh --template "![image]({remote_path})" -- user@example.com
```

把图片粘贴快捷键改为 `Ctrl+Shift+V`：

```powershell
ipssh --paste-hotkey "ctrl+shift+v" -- user@example.com
```

使用其他配置文件：

```powershell
ipssh --config C:\Users\me\ipssh.toml -- user@example.com
```

## SSH 认证

`ipssh` 把认证交给 OpenSSH。它不询问、保存或复用密码。

推荐：

- 把主机设置写入 `%USERPROFILE%\.ssh\config`。
- 使用密钥认证。
- 使用 `ssh-agent` 或 OpenSSH 连接复用，减少上传时重复输入密码。

## 故障排查

### 找不到 `ipssh`

安装后请打开新的终端。如果仍然失败，检查以下目录是否存在并位于用户 `PATH` 中：

```text
%LOCALAPPDATA%\Programs\ipssh
```

也可以直接运行：

```powershell
& "$env:LOCALAPPDATA\Programs\ipssh\ipssh.exe" --help
```

### 图片粘贴没有上传

检查：

- 剪贴板中是图片数据，而不是文件路径或 HTML 图片引用。
- 配置的快捷键与你按下的一致。
- `scp.exe` 在 `PATH` 中。
- 普通 `ssh` 和 `scp` 能连接同一目标。
- 远端目录可写。

### 每次上传都要求密码

这是 OpenSSH 行为。`ipssh` 上传时会启动独立的 `ssh` 和 `scp` 子进程。请使用密钥认证、`ssh-agent` 或 OpenSSH 连接复用减少重复提示。

### `Ctrl+D` 退出远端 shell

这是正常 SSH 行为。远端 shell 退出后，`ipssh` 会回到 Windows 命令提示符。

## 从源码构建

构建 CLI：

```powershell
cargo build --release
```

构建 Windows 安装包：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

运行测试：

```powershell
cargo test
$env:IPSSH_BIN='D:\git\image-paste-ssh\target\release\ipssh.exe'; cargo test --manifest-path packaging\windows\installer\Cargo.toml
```
