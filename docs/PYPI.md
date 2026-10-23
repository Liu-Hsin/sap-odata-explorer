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
每个 OS / 架构各需要一份：

| 平台 | wheel 标签 |
|---|---|
| Windows x64 | `py3-none-win_amd64` |
| Linux x64 | `py3-none-manylinux_2_17_x86_64` |
| macOS Apple Silicon | `py3-none-macosx_11_0_arm64` |
| macOS Intel | `py3-none-macosx_10_12_x86_64` |

**只发 Windows 那份的话，Linux/macOS 用户 `pip install` 会报
"No matching distribution found"。** 所以必须走 CI 矩阵，见第 4 节。

wheel 标签是 `py3-none-<platform>`——不含 Python ABI 绑定，因此任何
Python 3 版本都能装同一个 wheel。

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
各自冒烟测试 `sap-odata --version` → 汇总上传 PyPI。

`--skip-existing` 让重跑幂等：某个平台失败时修好重推 tag，已传的不会冲突。

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

### 5.1 Linux/macOS 上系统钥匙串不可用

`crates/core` 依赖 `keyring`，而 workspace 里只启用了 `windows-native` feature：

```toml
keyring = { version = "3", features = ["windows-native"] }
```

后果：非 Windows 平台**能编译、能装、命令能跑**，但保存 SAP 凭据到系统
钥匙串会失败（keyring 找不到可用 backend）。

修法（按平台条件启用）：

```toml
[target.'cfg(windows)'.dependencies]
keyring = { version = "3", features = ["windows-native"] }

[target.'cfg(target_os = "macos")'.dependencies]
keyring = { version = "3", features = ["apple-native"] }

[target.'cfg(target_os = "linux")'.dependencies]
keyring = { version = "3", features = ["sync-secret-service"] }
```

这不是打包问题，是功能完整性问题。如果首发只面向 Windows 用户，
可以暂时只发 Windows wheel，等其他平台验证过再补。

### 5.2 体积

wheel 约 3 MB（strip 后），安装后 `sap-odata.exe` 约 7 MB。
CI 的 `--release` 已开 `strip = true`。

### 5.3 pip 要求 Python 版本

`requires-python = ">=3.8"`。虽然二进制与 Python 版本无关，但 pip 需要这个
下限才能选 wheel。若目标用户可能用更老的 Python，改这一行即可，不必重新编译。

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
