# 发布到 PyPI（pip 安装）

本项目的 CLI 是 Rust 二进制。通过 [maturin](https://www.maturin.rs/) 的
`bindings = "bin"` 模式，把预编译产物打成**平台 wheel** 发布到 PyPI。

用户侧体验：

```bash
pip install sap-odata-cli
sap-odata --version        # 0.2.0
```

用户机器**不需要 Rust 工具链**，pip 按平台自动选择对应 wheel。

---

## 1. 包名与命令

| 项目 | 值 |
|---|---|
| PyPI 包名 | `sap-odata-cli` |
| 安装后命令 | `sap-odata` |
| crates.io 包名 | `sap-odata-cli`（已发布，两者同名不冲突） |
| 版本来源 | `Cargo.toml` 的 `workspace.package.version`（配置里是 `dynamic`） |

PyPI 包名是全球唯一的，注册后不能改名、不能删除（只能 yank）。
`pyproject.toml` 里的 `name` 就是最终包名，发布前务必确认。

同一份代码同时发 crates.io 和 PyPI 是常规做法，两个生态的命名空间互不干扰。
`pip install sap-odata-cli` 拿预编译二进制，`cargo install sap-odata-cli` 从源码编译，
互不影响。

---

## 2. wheel 的平台矩阵

`bindings = "bin"` 产生的是**平台相关** wheel（`Root-Is-Purelib: false`），
每个 OS / 架构各需要一份。`release-pypi.yml` 覆盖这四个目标：

| 平台 | Rust target | 构建 runner | wheel 标签 |
|---|---|---|---|
| Windows x64 | `x86_64-pc-windows-msvc` | `windows-latest` | `py3-none-win_amd64` |
| Linux x64 | `x86_64-unknown-linux-gnu` | `ubuntu-latest` + manylinux_2_28 容器 | `py3-none-manylinux_2_28_x86_64` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `macos-14` | `py3-none-macosx_11_0_arm64` |
| macOS Intel | `x86_64-apple-darwin` | `macos-14`（交叉编译） | `py3-none-macosx_10_12_x86_64` |

wheel 标签是 `py3-none-<platform>`——不含 Python ABI 绑定，任何 Python 3
版本都装同一个 wheel。Linux 份要求 **glibc ≥ 2.28**（RHEL 8 / Ubuntu 20.04
起步），要兼容更老的发行版就把 `manylinux` 降回 `2014`，但那个镜像是
CentOS 7、包源不稳定。

**这四份只能在 CI 里产出。** Windows 本机能编的只有第一份：交叉编译
Linux 需要 Docker / WSL，macOS 需要 Apple SDK，两者在 Windows 上都不成立。
所以本地 `build-wheel.ps1` 的价值是快速验证，不是凑齐发布物。

**只发一部分会怎样**：缺哪个平台，那个平台的用户 `pip install` 就报
`No matching distribution found`——搜得到、装不上。四个平台齐全再发，
比发一半体验好。

### 2.1 两个跨平台细节（已处理，改动时别退回去）

| 点 | 处理方式 |
|---|---|
| 系统钥匙串 | `keyring` 按平台分别启用 backend（详见 5.1）。漏掉会**静默丢密码**，比报错更糟 |
| Linux 的 OpenSSL | 用 `openssl` 的 `vendored` feature 静态链接。否则 wheel 会绑定构建机上的 `libssl.so.X`，在 RHEL 8 / Ubuntu 20.04 上直接起不来 |
| 容器里的 perl | `openssl-src` 用 `perl ./Configure` 配置内置 OpenSSL，而 manylinux 镜像的 perl 被裁到只剩核心。工作流里用 `before-script-linux` 探测并补装 `perl-IPC-Cmd`、`perl-Time-Piece` |

Linux 包仍然依赖 glibc，具体下限看 wheel 名里的 `manylinux_*` 标签。

---

## 3. 首次发布准备

### 3.1 注册 PyPI 并开 2FA

1. 注册 <https://pypi.org/account/register/>
2. 账号设置里开启 **two-factor authentication**（现在强制）

### 3.2 配置 Trusted Publishing（推荐，无需 API token）

到 <https://pypi.org/manage/account/publishing/> 填一个 **pending publisher**：

| 字段 | 值 |
|---|---|
| PyPI Project Name | `sap-odata-cli` |
| Owner | 实际推送的 GitHub 账号（`Cargo.toml` 里写的是 `kts982`） |
| Repository name | `sap-odata-explorer` |
| Workflow name | `release-pypi.yml` |
| Environment name | `pypi` |

这样 CI 用 OIDC 令牌直接发布，**不用在仓库里存任何密钥**。
GitHub 仓库的 Settings → Environments 里建一个名为 `pypi` 的环境即可。

### 3.3 备选：API token 方式

不想用 OIDC 的话：

1. <https://pypi.org/manage/account/token/> 创建 token（scope 选项目级）
2. GitHub 仓库 Secrets 里存为 `PYPI_API_TOKEN`
3. 把 workflow 的 publish job 改成：

```yaml
- name: Publish to PyPI
  uses: pypa/gh-action-pypi-publish@release/v1
  with:
    packages-dir: dist/
    skip-existing: true
    password: ${{ secrets.PYPI_API_TOKEN }}
```

并把该 job 的 `permissions: id-token: write` 去掉。

> **为什么不用 `maturin upload`**：maturin 已废弃 `upload` / `publish` 命令
> （[PyO3/maturin#2334](https://github.com/PyO3/maturin/issues/2334)），
> 官方建议改用 `twine` 或 `uv publish`。CI 里用 PyPA 官方的
> `gh-action-pypi-publish`，它同时处理 OIDC 和 attestations。

---

## 4. 发布流程

### 4.1 通过 CI（正式方式）

```bash
# 1) 确认版本号已提升（Cargo.toml 的 workspace.package.version）
git tag v0.2.0
git push origin v0.2.0
```

推 tag 触发 `.github/workflows/release-pypi.yml`：四个平台并行构建 →
各自冒烟测试（装进干净 venv 跑 `sap-odata --version` 和 `--help`）→
汇总上传 PyPI。

`--skip-existing` 让重跑幂等：某个平台失败时修好重推 tag，已传的不会冲突。

> **建 GitHub Release 也等于推了一次 tag。** 在网页或 API 上创建 Release 时
> 顺手创建的标签同样会触发本流水线，并走到 publish 任务——也就是"本想发个
> Release，结果包也上了 PyPI"。发版前给仓库的 `pypi` environment 配上
> **required reviewers**，任何自动化发布都会先停在等人批准，这是唯一可靠的闸门。

### 4.1.1 只构建、不发布（先凑齐四份 wheel）

不想立刻发 PyPI，只想拿到四个平台的包：仓库 Actions →
`Release to PyPI` → **Run workflow**，`publish` 保持不勾选。

跑完在该次 run 的页面底部 **Artifacts** 里能下到四个压缩包：

```
wheels-x86_64-pc-windows-msvc.zip
wheels-x86_64-unknown-linux-gnu.zip
wheels-aarch64-apple-darwin.zip
wheels-x86_64-apple-darwin.zip
```

四份解压到同一个 `dist/` 就是完整的发布集，可手工 `twine upload`
或留着等下次推 tag 一起发。

> `publish` 勾上则需要仓库已配好 `pypi` environment 和 trusted publisher，
> 否则 publish job 会被跳过或校验失败。

### 4.2 本地发布（仅调试用）

本仓库带两个脚本，自动定位 maturin / twine，**不需要把它们加进 PATH**：

```powershell
./scripts/build-wheel.ps1                                       # 构建当前平台 wheel
./scripts/pypi-upload.ps1 -Target testpypi -Token "pypi-..."    # 先 check 再上传
```

- `build-wheel.ps1` 查找顺序：PATH → 本机工具目录 → `~/.cargo/bin`，
  找不到会提示安装方式。构建完打印 wheel 全路径和本地安装命令。
- `pypi-upload.ps1` 强制先跑 `twine check`，不通过就不上传；
  没给 token 直接退出，不会误传。`-Target` 可选 `testpypi` / `pypi`。

下面是手动步骤，脚本内部做的就是这些。

---

本地只能构建当前平台的 wheel。分工是：**maturin 负责 build，twine 负责 upload**
（maturin 已废弃自带的 `upload` 命令）。

```powershell
maturin build --release --out dist
python -m twine check dist/*.whl
```

`twine check` 必须返回 `PASSED`，它会验证元数据完整性和 README 渲染。
没装 twine 就先 `pip install twine`。

**认证只能用 API token** —— PyPI 和 TestPyPI 都已停用用户名密码，
用真名登录会直接返回：

```
403 Username/Password authentication is no longer supported.
```

TestPyPI 与正式 PyPI 是**两套独立账号**，需分别注册：

1. <https://test.pypi.org/account/register/> 注册
2. <https://test.pypi.org/manage/account/token/> 创建 token
   （项目首次发布前只能选 **Entire account** 范围）
3. 复制 `pypi-...` 开头的字符串

上传到 TestPyPI：

```powershell
$env:TWINE_USERNAME = "__token__"
$env:TWINE_PASSWORD = "pypi-AgENdGVzdC5weXBpLm9yZw..."
python -m twine upload --repository-url https://test.pypi.org/legacy/ --skip-existing dist/*.whl
```

或显式传参（便于临时切换 token）：

```powershell
python -m twine upload --repository-url https://test.pypi.org/legacy/ `
  --username __token__ --password "pypi-AgENdGVzdC5weXBpLm9yZw..." dist/*.whl
```

四个易错点：

| 症状 | 原因 |
|---|---|
| `403 Username/Password authentication is no longer supported` | `--username` 没写 `__token__`，或密码位填了账号密码 |
| `403 Invalid or non-existent authentication information` | 通道对了，但 token 无效／过期／属于另一个站点（TestPyPI 和 PyPI 的 token 不通用） |
| `405 Not Allowed` / 连接被拒 | 上传地址漏了 `legacy/` 后缀 |
| `No matching distribution found`（安装时） | 只加了 TestPyPI 索引，依赖不在那上面 |

装包验证时必须带双索引：

```powershell
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ sap-odata-cli
```

想直发正式源，把地址换成 `https://upload.pypi.org/legacy/`：

```powershell
$env:TWINE_PASSWORD = "pypi-..."   # 换成 pypi.org 的 token
python -m twine upload --repository-url https://upload.pypi.org/legacy/ dist/*.whl
```

> **注意**：同一个版本号上传后**不可覆盖**。改代码后必须提升版本号，
> 否则 PyPI 直接拒绝。本地反复测试请用 TestPyPI。
>
> TestPyPI 会定期清理旧包，**不要把它当长期分发渠道**，它只用于验证链路。
>
> 用完记得 `Remove-Item Env:TWINE_PASSWORD`，别把 token 留在终端历史里。

---

## 5. 已知限制

### 5.1 系统钥匙串：按平台挂不同的 backend

`keyring` 3.x **没有默认 backend feature**。一个都不开时它会编译进
`mock` 存储——`set_password` 返回成功，密码却只存在 `Entry` 对象里，
进程一退出就没了。所以不能只在 workspace 里写 `windows-native`。

现在的接法（`crates/core/Cargo.toml`，按 target 分别声明）：

| 平台 | feature | 实际后端 |
|---|---|---|
| Windows | `windows-native` | 凭据管理器 |
| macOS | `apple-native` | Keychain |
| Linux | `linux-native` | 内核 keyutils |
| 其他 unix | 无 | `mock`（仅保证能编译运行） |

Linux 选 `linux-native` 是为了构建可靠：纯 syscall，不需要 libdbus，
manylinux 容器里一定能编过。代价是凭据存在**登录会话**里，注销或重启后
需要重新输入密码。

想要跨重启持久化，换成 Secret Service：

```toml
[target.'cfg(target_os = "linux")'.dependencies]
keyring = { version = "3", features = ["sync-secret-service", "crypto-rust"] }
```

代价是构建时要能链上 libdbus（`dbus-secret-service` 加 `vendored`
feature 可静态编译），且运行时要有一个可用的 D-Bus 会话（GNOME Keyring /
KWallet）。无头服务器、容器里通常没有，会退化成报错。

判断是否踩了 mock 坑：在目标机上跑 `profile add` 存密码，然后**另开一个
进程**跑 `services`。如果又提示要密码，就是这个 feature 没配对。

### 5.2 体积

wheel 约 3 MB（strip 后），安装后 `sap-odata.exe` 约 7 MB。
CI 的 `--release` 已开 `strip = true`。

### 5.3 pip 要求 Python 版本

`requires-python = ">=3.8"`。虽然二进制与 Python 版本无关，但 pip 需要这个
下限才能选 wheel。若目标用户可能用更老的 Python，改这一行即可，不必重新编译。

### 5.4 装完敲 `sap-odata` 提示"不是内部或外部命令"

`pip install` 把 exe 放进 **user Scripts** 目录，而这个目录通常不在 PATH 里。

常见位置：

| Python 来源 | user Scripts 路径 |
|---|---|
| Microsoft Store | `%LOCALAPPDATA%\Packages\PythonSoftwareFoundation.Python.3.13_qbz5n2kfra8p0\LocalCache\local-packages\Python313\Scripts` |
| python.org 安装包 | `%APPDATA%\Python\Python313\Scripts` |

查实际路径（任何环境都准）：

```powershell
python -c "import site,os; print(os.path.join(site.USER_BASE,'Scripts'))"
```

把它加进**用户级** PATH：

```powershell
$s   = python -c "import site,os; print(os.path.join(site.USER_BASE,'Scripts'))"
$old = [Environment]::GetEnvironmentVariable('Path','User')
[Environment]::SetEnvironmentVariable('Path', "$old;$s", 'User')
```

重开终端生效。

> **不要用 `setx PATH "%PATH%;..."`**——它会把系统 PATH 一并并入用户变量，
> 造成重复条目并可能撑爆 1024 字符上限，之后排查起来很麻烦。

不想动 PATH 的话，直接用全路径调用，或者改用 `pipx install sap-odata-cli`
（pipx 会自己处理隔离环境和入口）。


---

## 6. 验证清单

发布后逐条过：

```bash
# 全新 venv，模拟真实用户
python -m venv /tmp/test-venv
/tmp/test-venv/bin/pip install sap-odata-cli

/tmp/test-venv/bin/sap-odata --version     # 应输出 0.2.0
/tmp/test-venv/bin/sap-odata --help        # 子命令完整

# 确认能从 PATH 直接调用
which sap-odata

# 卸载干净
/tmp/test-venv/bin/pip uninstall -y sap-odata-cli
```

PyPI 页面同步检查：<https://pypi.org/project/sap-odata-cli/>
（README 渲染、分类标签、License、文件列表里的四个平台 wheel）。
