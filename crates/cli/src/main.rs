use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{Cell, Color, Table, presets::UTF8_FULL};
use sap_odata_core::{
    auth::{AuthConfig, SapConnection},
    client::SapClient,
    config::{self, ConnectionProfile},
    diagnostics::HttpTraceEntry,
    lint::{self, LintSeverity},
    metadata::ODataVersion,
    offline::{
        self, ImportOptions, SaveOptions, SaveOutcome, auto_offline_profile_name, current_iso8601,
    },
    query::ODataQuery,
};

#[derive(Parser)]
#[command(
    name = "sap-odata",
    version,
    about = "SAP OData 服务浏览器——浏览实体、构建查询并测试服务"
)]
struct Cli {
    /// 连接配置名称（来自 connections.toml）
    #[arg(long, short = 'p', global = true)]
    profile: Option<String>,

    /// SAP 系统基础 URL（覆盖配置文件中的值）
    #[arg(long, env = "SAP_BASE_URL")]
    url: Option<String>,

    /// SAP 客户端编号（覆盖配置文件中的值）
    #[arg(long, env = "SAP_CLIENT")]
    client: Option<String>,

    /// 语言代码（覆盖配置文件中的值）
    #[arg(long, env = "SAP_LANGUAGE")]
    language: Option<String>,

    /// Basic 认证用户名（覆盖配置文件中的值）
    #[arg(long, env = "SAP_USER", hide_env_values = true)]
    user: Option<String>,

    /// Basic 认证密码（覆盖配置文件中的值）
    // hide_env_values: without it, `--help` prints `[env: SAP_PASSWORD=<value>]`.
    #[arg(long, env = "SAP_PASSWORD", hide_env_values = true)]
    password: Option<String>,

    /// OData 服务路径（例如 /sap/opu/odata/sap/ZMY_SERVICE_SRV）。
    /// 除 services 和 profile 外，其他命令均需要此参数。
    #[arg(long, short = 's')]
    service: Option<String>,

    /// 以 JSON 格式输出，而不是表格
    #[arg(long, global = true)]
    json: bool,

    /// 显示调试日志
    #[arg(long, short = 'v', global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 输出内置的 AI agent skill 文档
    Skill,

    /// 通过交互式向导添加 SAP 系统
    Setup,

    /// 清除指定配置的 Browser SSO 会话
    Signout {
        /// 配置名称
        name: String,
    },

    /// 管理连接配置
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },

    /// 管理连接配置的服务别名（需要 -p）
    Alias {
        #[command(subcommand)]
        action: AliasAction,
    },

    /// 列出 SAP Gateway 服务目录中的所有 OData 服务
    Services {
        /// 按名称、标题或描述筛选服务（不区分大小写）
        #[arg(long, short = 'f')]
        filter: Option<String>,

        /// 仅显示 V2 服务
        #[arg(long)]
        v2: bool,

        /// 仅显示 V4 服务
        #[arg(long)]
        v4: bool,

        /// 最多显示的服务数量
        #[arg(long)]
        top: Option<usize>,
    },

    /// 列出服务中的所有实体集
    Entities,

    /// 描述实体类型——显示属性、键和导航属性
    Describe {
        /// 实体集名称（例如 CustomerSet）
        entity_set: String,
    },

    /// 列出服务中的函数导入
    Functions,

    /// 构建并显示 OData 查询 URL（不发送请求）
    #[command(long_about = "构建并显示 OData 查询 URL，但不发送请求。\n\
        \n\
        常用参数（`run` 命令也支持）：\n  \
            --select 字段列表       以逗号分隔的 $select 字段\n  \
            --filter 表达式         原始 OData $filter 表达式\n  \
            --in 属性 值...         OR 等值快捷写法，生成 (属性 eq '值1' or 属性 eq '值2' ...)\n  \
            --expand 导航属性       以逗号分隔的 $expand 导航属性\n  \
            --orderby 排序子句      以逗号分隔的 $orderby 子句（例如 \"Name asc,Date desc\"）\n  \
            --top 数量              $top\n  \
            --skip 数量             $skip\n  \
            --key 键值              读取单个实体：CustomerSet(K)\n  \
            --count                 请求内联计数\n  \
            --json                  将 URL 作为 JSON 字符串输出，而不是纯文本")]
    Build {
        /// 实体集名称
        entity_set: String,

        /// 要选择的字段（以逗号分隔）
        #[arg(long)]
        select: Option<String>,

        /// 筛选表达式
        #[arg(long)]
        filter: Option<String>,

        /// OR 等值快捷写法。用法：`--in PROP V1 V2 V3` 生成
        /// `(PROP eq 'V1' or PROP eq 'V2' or PROP eq 'V3')`。与 `--filter`
        /// 通过 AND 组合。仅支持字符串值，程序会自动添加引号。
        #[arg(long = "in", num_args = 2.., value_name = "PROP VALUES...")]
        in_values: Vec<String>,

        /// 要展开的导航属性（以逗号分隔）
        #[arg(long)]
        expand: Option<String>,

        /// 排序子句（以逗号分隔，例如 "Name asc,Date desc"）
        #[arg(long)]
        orderby: Option<String>,

        /// 最大结果数
        #[arg(long)]
        top: Option<u32>,

        /// 跳过的结果数
        #[arg(long)]
        skip: Option<u32>,

        /// 实体键（例如 "'CUST01'" 或 "CustomerID='CUST01',CompanyCode='1000'"）
        #[arg(long)]
        key: Option<String>,

        /// 请求内联计数
        #[arg(long)]
        count: bool,
    },

    /// 执行 OData 查询并显示结果
    #[command(long_about = "执行 OData 查询并显示结果。\n\
        \n\
        常用参数：\n  \
            --select 字段列表       以逗号分隔的 $select 字段\n  \
            --filter 表达式         原始 OData $filter 表达式\n  \
            --in 属性 值...         OR 等值快捷写法，生成 (属性 eq '值1' or 属性 eq '值2' ...)\n  \
            --expand 导航属性       以逗号分隔的 $expand 导航属性\n  \
            --orderby 排序子句      以逗号分隔的 $orderby 子句\n  \
            --top 数量              $top（默认由服务端限制）\n  \
            --skip 数量             $skip（分页）\n  \
            --key 键值              读取单个实体\n  \
            --count                 请求内联计数\n  \
            --json                  输出原始响应 JSON，而不是表格\n\
        \n\
        如需仅生成 URL 而不发送请求，请改用 `build`。")]
    Run {
        /// 实体集名称
        entity_set: String,

        /// 要选择的字段（以逗号分隔）
        #[arg(long)]
        select: Option<String>,

        /// 筛选表达式
        #[arg(long)]
        filter: Option<String>,

        /// OR 等值快捷写法。用法：`--in PROP V1 V2 V3` 生成
        /// `(PROP eq 'V1' or PROP eq 'V2' or PROP eq 'V3')`，并通过 AND 与 `--filter` 组合。
        #[arg(long = "in", num_args = 2.., value_name = "PROP VALUES...")]
        in_values: Vec<String>,

        /// 要展开的导航属性（以逗号分隔）
        #[arg(long)]
        expand: Option<String>,

        /// 排序子句
        #[arg(long)]
        orderby: Option<String>,

        /// 最大结果数
        #[arg(long)]
        top: Option<u32>,

        /// 跳过的结果数
        #[arg(long)]
        skip: Option<u32>,

        /// 实体键
        #[arg(long)]
        key: Option<String>,

        /// 请求内联计数
        #[arg(long)]
        count: bool,
    },

    /// 获取并显示原始 $metadata XML
    Metadata,

    /// 对服务中的每个实体集进行冒烟测试。分别发送小型 `$top` GET 请求，
    /// 并报告成功/失败及行数。全部探测成功时退出码为 0，否则为 1，
    /// 可用于 CI 健康检查。
    #[command(long_about = "对服务中的每个实体集进行冒烟测试。\n\
        \n\
        分别对每个实体集发送小型 `$top` GET 请求，并报告成功/失败及行数。\n\
        SAP V4 框架实体集（以 SAP__/__ 开头）会被跳过，因为它们属于运行时结构而非业务数据。\n\
        \n\
        参数：\n  \
            --quick                仅列出实体集，不逐个发送 GET 请求\n  \
            --top 数量             每次探测使用的 $top（默认 1）\n  \
            --json                 以 JSON 数组输出结果\n\
        \n\
        退出码：全部探测成功时为 0，任一探测失败时为 1。")]
    Verify {
        /// 仅列出实体集，跳过逐个探测
        #[arg(long)]
        quick: bool,

        /// 每次探测使用的 $top。数值越小越快；默认值 1 足以确认实体集可访问。
        #[arg(long, default_value_t = 1)]
        top: u32,
    },

    /// 检查实体是否满足 Fiori 使用要求。根据 Fiori 列表报表/对象页通常需要的
    /// 服务注解进行评估（HeaderInfo、LineItem、SelectionFields、Text/Unit 关联、Capabilities 等）。
    /// 可指定单个实体；省略参数则扫描整个服务。
    Lint {
        /// 实体类型或实体集名称。两种名称均可；省略则检查服务中的所有实体。
        entity: Option<String>,

        /// 仅显示达到此严重级别的结果（pass / warn / miss），默认显示全部。
        #[arg(long)]
        min_severity: Option<String>,

        /// 覆盖自动检测的 lint 配置类型。适用于启发式判断不准确时；
        /// 例如实体形状符合列表报表，但名称不符合约定。使用 --lint-profile
        /// 以区别全局连接配置参数（-p、--profile）。
        #[arg(long = "lint-profile", value_enum)]
        lint_profile: Option<LintProfileArg>,

        /// 存在达到此严重级别的结果时以非零状态退出，适用于 CI。
        /// 此参数独立于 --min-severity，后者仅筛选显示内容。
        #[arg(long, value_enum)]
        fail_on: Option<FailOnArg>,
    },

    /// 列出从 $metadata 解析出的所有 SAP/UI5 注解，并按词汇命名空间分组。
    /// 可用于确认服务是否声明了某项注解，而无需重新查看原始 XML。
    Annotations {
        /// 按命名空间筛选（例如 "UI"、"Common"、"Capabilities"），不区分大小写；省略则列出全部。
        #[arg(long)]
        namespace: Option<String>,

        /// 在 Term、Target、Value 和 Qualifier 字段中进行子串匹配（不区分大小写）。
        #[arg(long)]
        filter: Option<String>,
    },

    /// 管理离线 EDMX 库（保存、列出或导入服务，以供离线浏览）
    Offline {
        #[command(subcommand)]
        action: OfflineAction,
    },
}

#[derive(Subcommand)]
enum OfflineAction {
    /// 将当前连接服务的 $metadata 保存到离线库。需要 -p PROFILE -s SERVICE。
    /// 文件写入 {config}/offline/ 并关联到当前连接配置；再次保存时会使用相同的 service_id 覆盖原文件。
    Save {
        /// 指定离线配置桶名称。默认使用 <连接配置名> (offline)。
        /// 若该名称尚不存在则会创建；不能与现有连接配置重名。
        #[arg(long)]
        offline_profile: Option<String>,

        /// 覆盖自动生成的显示标签。V4 SAP 服务会根据 Schema 命名空间生成标签，
        /// 例如 UI_PHYSSTOCKPROD_1。
        #[arg(long)]
        label: Option<String>,

        /// 附加到保存条目的备注（可选）
        #[arg(long)]
        note: Option<String>,
    },

    /// 将磁盘上的 EDMX 文件导入离线库。无需连接配置；程序会读取、验证并索引文件。
    /// 可导入从 API Hub 获取、由 /IWFND/GW_CLIENT 的“Save Response”导出，
    /// 或通过 curl <base>/$metadata > out.xml 下载的文件。
    Import {
        /// 磁盘上的 EDMX 文件路径。必须是普通文件、UTF-8 编码且不超过 10 MB。
        file: std::path::PathBuf,

        /// 目标离线配置桶。默认为 Imported，用于存放来自不同来源的文件。
        #[arg(long)]
        offline_profile: Option<String>,

        /// 覆盖自动生成的显示标签。默认读取文件中的 Schema Namespace；
        /// 无法识别时使用文件名（不含扩展名）。
        #[arg(long)]
        label: Option<String>,

        /// 附加到导入条目的备注（可选）
        #[arg(long)]
        note: Option<String>,
    },

    /// 列出离线配置桶中的所有服务。未指定 --profile 时，改为列出所有离线配置。
    List {
        /// 要列出服务的离线配置桶名称；省略则列出配置桶。
        #[arg(long)]
        profile: Option<String>,
    },

    /// 删除离线服务或整个离线配置桶。指定 --service-id 时仅删除单个服务（文件和索引记录）；
    /// 否则删除整个配置桶及其缓存文件。只删除索引记录的文件，不会递归删除目录。
    Delete {
        /// 离线配置桶名称（必填）
        #[arg(long)]
        profile: String,

        /// 要删除的服务 ID。省略则删除整个配置桶。使用 `offline list --profile NAME` 查看 ID。
        #[arg(long)]
        service_id: Option<String>,

        /// 跳过确认提示，适用于脚本
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

/// Lint profile override flag values. Mirrors `lint::LintProfile`
/// but is decoupled so clap's value-enum derive owns the kebab-case
/// CLI surface (`list-report`, `object-page`, …) without leaking
/// clap into the core crate.
#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum LintProfileArg {
    ListReport,
    ObjectPage,
    ValueHelp,
    Analytical,
    Transactional,
}

impl From<LintProfileArg> for lint::LintProfile {
    fn from(value: LintProfileArg) -> Self {
        match value {
            LintProfileArg::ListReport => lint::LintProfile::ListReport,
            LintProfileArg::ObjectPage => lint::LintProfile::ObjectPage,
            LintProfileArg::ValueHelp => lint::LintProfile::ValueHelp,
            LintProfileArg::Analytical => lint::LintProfile::Analytical,
            LintProfileArg::Transactional => lint::LintProfile::Transactional,
        }
    }
}

/// `--fail-on` accepts the actionable severities only — `pass` would
/// fire on every entity and isn't a useful CI gate.
#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "lowercase")]
enum FailOnArg {
    Warn,
    Miss,
}

impl From<FailOnArg> for LintSeverity {
    fn from(value: FailOnArg) -> Self {
        match value {
            FailOnArg::Warn => LintSeverity::Warn,
            FailOnArg::Miss => LintSeverity::Miss,
        }
    }
}

#[derive(Subcommand)]
enum ProfileAction {
    /// 列出所有已保存的连接配置
    List,

    /// 添加或更新连接配置
    Add {
        /// 配置名称（例如 DEV、QAS、PRD）
        name: String,

        /// SAP 系统基础 URL
        #[arg(long)]
        url: String,

        /// SAP 客户端编号
        #[arg(long, default_value = "100")]
        client: String,

        /// 语言代码
        #[arg(long, default_value = "EN")]
        language: String,

        /// 用户名（使用 --sso 时不需要）
        #[arg(long, default_value = "")]
        user: String,

        /// 密码（使用 --sso 时不需要；默认存入操作系统凭据库）
        #[arg(long, default_value = "")]
        password: String,

        /// 使用 Windows SSO（SPNEGO/Kerberos），无需用户名和密码
        #[arg(long)]
        sso: bool,

        /// 启用 --sso 时允许 Kerberos 委派（服务器可使用用户身份访问下游后端）。
        /// 默认关闭；仅在反向代理 → Gateway → 后端 R/3 需要多跳认证时启用。
        #[arg(long)]
        sso_delegate: bool,

        /// 将密码存入配置文件而不是操作系统凭据库（不推荐）
        #[arg(long)]
        plaintext: bool,

        /// 在可执行文件旁创建便携式配置
        #[arg(long)]
        portable: bool,

        /// 系统环境：DEV、QAS 或 PRD（none 用于清除）。PRD 配置会在桌面应用中标记，
        /// 并在标准错误中提示。更新配置时省略此参数会保留原值。
        #[arg(long = "env", value_name = "DEV|QAS|PRD")]
        environment: Option<String>,
    },

    /// 删除连接配置
    Remove {
        /// 要删除的配置名称
        name: String,
    },

    /// 测试连接配置
    Test {
        /// 要测试的配置名称
        name: String,
    },

    /// 显示配置文件所在位置
    Where,
}

#[derive(Subcommand)]
enum AliasAction {
    /// 列出连接配置的所有别名
    List,

    /// 添加或更新服务别名
    Add {
        /// 别名（例如 warehouse、orders）
        name: String,

        /// 完整的 OData 服务路径
        path: String,
    },

    /// 删除服务别名
    Remove {
        /// 要删除的别名
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if matches!(&cli.command, Commands::Skill) {
        print!("{}", include_str!("../../../skills/sap-odata-cli/SKILL.md"));
        return Ok(());
    }

    let log_level = if cli.verbose {
        "sap_odata=debug"
    } else {
        "sap_odata=warn"
    };
    // Logs go to stderr: stdout carries only command output, so
    // `--json` stays parseable with `-v` (and when a warning fires).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(log_level.parse().unwrap()),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    // Handle commands that don't need a SAP connection
    if let Commands::Profile { action } = &cli.command {
        return handle_profile_command(action, cli.json).await;
    }
    if let Commands::Alias { action } = &cli.command {
        return handle_alias_command(action, cli.profile.as_deref(), cli.json);
    }
    if matches!(&cli.command, Commands::Setup) {
        return cmd_setup_wizard().await;
    }
    if let Commands::Signout { name } = &cli.command {
        return cmd_signout(name);
    }
    // `offline import` reads a local file and doesn't talk to SAP —
    // skip the connection-resolution step that follows so the user
    // can import without any profile / URL / credentials configured.
    if let Commands::Offline {
        action:
            OfflineAction::Import {
                file,
                offline_profile,
                label,
                note,
            },
    } = &cli.command
    {
        return cmd_offline_import(
            file.clone(),
            offline_profile.clone(),
            label.clone(),
            note.clone(),
            cli.json,
        );
    }
    // `offline list` reads only the local TOML index — no network.
    if let Commands::Offline {
        action: OfflineAction::List { profile },
    } = &cli.command
    {
        return cmd_offline_list(profile.clone(), cli.json);
    }
    // `offline delete` mutates the local TOML index + on-disk EDMX
    // files; never touches the network. Same early-exit as import /
    // list so the user can delete buckets without a configured
    // connection.
    if let Commands::Offline {
        action:
            OfflineAction::Delete {
                profile,
                service_id,
                yes,
            },
    } = &cli.command
    {
        return cmd_offline_delete(profile.clone(), service_id.clone(), *yes, cli.json);
    }

    // `-p` naming an offline bucket: metadata commands read the cached
    // EDMX — no connection, no network.
    if let Some(ref name) = cli.profile {
        let (cfg, config_dir) = config::load_config().context("加载配置文件失败")?;
        if cfg.offline_profiles.contains_key(name) && !cfg.connections.contains_key(name) {
            return run_offline_bucket_command(&cli, &cfg, &config_dir.path, name).await;
        }
    }

    // Resolve connection: profile → env vars → CLI flags
    let connection = resolve_connection(&cli)?;
    let uses_browser_sso = matches!(&connection.auth, AuthConfig::Browser);
    let sap_client = SapClient::new(connection).context("创建 SAP 客户端失败")?;

    // For Browser SSO profiles, try to load persisted cookies from keyring.
    // If none are present, fail early with a clear message.
    if uses_browser_sso && let Some(ref profile_name) = cli.profile {
        let loaded = sap_client
            .try_load_persisted_session(profile_name)
            .context("读取已保存的会话失败")?;
        if !loaded {
            anyhow::bail!(
                "Browser SSO 配置 '{}' 没有有效会话。\n\
                     请打开桌面应用，选择此配置并点击“登录”，\n\
                     然后重新运行命令。",
                profile_name
            );
        }
    }

    let result = async {
        // Resolve service path: alias -> catalog lookup -> raw path
        let resolved_service = resolve_service_path(&cli, &sap_client).await?;

        let require_service = || -> Result<String> {
            resolved_service
                .clone()
                .ok_or_else(|| anyhow::anyhow!("此命令需要指定 --service / -s"))
        };

        match &cli.command {
            Commands::Profile { .. }
            | Commands::Alias { .. }
            | Commands::Skill
            | Commands::Setup
            | Commands::Signout { .. } => unreachable!(),
            Commands::Services {
                filter,
                v2,
                v4,
                top,
            } => cmd_services(&sap_client, filter.as_deref(), *v2, *v4, *top, cli.json).await,
            Commands::Entities => {
                let svc = require_service()?;
                cmd_entities(&MetaSource::Live { client: &sap_client, service: &svc }, cli.json).await
            }
            Commands::Describe { entity_set } => {
                let svc = require_service()?;
                cmd_describe(&MetaSource::Live { client: &sap_client, service: &svc }, entity_set, cli.json).await
            }
            Commands::Functions => {
                let svc = require_service()?;
                cmd_functions(&MetaSource::Live { client: &sap_client, service: &svc }, cli.json).await
            }
            Commands::Build {
                entity_set,
                select,
                filter,
                in_values,
                expand,
                orderby,
                top,
                skip,
                key,
                count,
            } => {
                let svc = require_service()?;
                let combined = combine_filter_with_in(filter.clone(), in_values)?;
                let query = build_query(
                    entity_set,
                    select.clone(),
                    combined,
                    expand.clone(),
                    orderby.clone(),
                    *top,
                    *skip,
                    key.clone(),
                    *count,
                    Some(version_from_service_path(&svc)),
                );
                let full_url = format!("{}/{}", svc.trim_end_matches('/'), query.build());
                if cli.json {
                    println!("{}", serde_json::to_string(&full_url)?);
                } else {
                    println!("{full_url}");
                }
                Ok(())
            }
            Commands::Run {
                entity_set,
                select,
                filter,
                in_values,
                expand,
                orderby,
                top,
                skip,
                key,
                count,
            } => {
                let svc = require_service()?;
                let meta = sap_client
                    .fetch_metadata(&svc)
                    .await
                    .context("获取元数据失败")?;
                let combined = combine_filter_with_in(filter.clone(), in_values)?;
                let query = build_query(
                    entity_set,
                    select.clone(),
                    combined,
                    expand.clone(),
                    orderby.clone(),
                    *top,
                    *skip,
                    key.clone(),
                    *count,
                    Some(meta.version),
                );
                cmd_run(&sap_client, &svc, &query, cli.json).await
            }
            Commands::Metadata => {
                let svc = require_service()?;
                cmd_metadata(&MetaSource::Live { client: &sap_client, service: &svc }).await
            }
            Commands::Verify { quick, top } => {
                let svc = require_service()?;
                cmd_verify(&sap_client, &svc, *quick, *top, cli.json).await
            }
            Commands::Annotations { namespace, filter } => {
                let svc = require_service()?;
                cmd_annotations(
                    &MetaSource::Live { client: &sap_client, service: &svc },
                    cli.json,
                    namespace.clone(),
                    filter.clone(),
                )
                .await
            }
            Commands::Lint {
                entity,
                min_severity,
                lint_profile,
                fail_on,
            } => {
                let svc = require_service()?;
                cmd_lint(
                    &MetaSource::Live { client: &sap_client, service: &svc },
                    cli.json,
                    entity.clone(),
                    min_severity.clone(),
                    *lint_profile,
                    *fail_on,
                )
                .await
            }
            Commands::Offline { action } => match action {
                OfflineAction::Save {
                    offline_profile,
                    label,
                    note,
                } => {
                    let svc = require_service()?;
                    let connected_profile = cli.profile.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "`offline save` 需要指定 `-p PROFILE`，以便将保存的元数据关联到对应的连接系统"
                        )
                    })?;
                    cmd_offline_save(
                        &sap_client,
                        &connected_profile,
                        &svc,
                        offline_profile.clone(),
                        label.clone(),
                        note.clone(),
                        cli.json,
                    )
                    .await
                }
                OfflineAction::Import { .. }
                | OfflineAction::List { .. }
                | OfflineAction::Delete { .. } => {
                    // All handled in the early-exit block above. If
                    // we reach here, the early-exit was skipped —
                    // that's a logic bug, not user error.
                    unreachable!(
                        "offline import/list/delete 应在连接解析前提前处理"
                    );
                }
            },
        }
    }
    .await;

    if cli.verbose {
        emit_http_trace(&sap_client);
    }

    result
}

fn emit_http_trace(client: &SapClient) {
    let trace = client.diagnostics_snapshot();
    if trace.is_empty() {
        return;
    }

    eprintln!();
    eprintln!("HTTP 请求跟踪（{} 次交互）：", trace.len());
    for entry in &trace {
        emit_http_trace_entry(entry);
    }
}

fn emit_http_trace_entry(entry: &HttpTraceEntry) {
    let status = entry
        .status
        .map(|status| status.to_string())
        .unwrap_or_else(|| "错误".to_string());

    eprintln!();
    eprintln!("[{}] {} {}", entry.id, entry.method, entry.url);
    eprintln!("  状态：{status}    耗时：{} 毫秒", entry.duration_ms);

    if let Some(location) = &entry.redirect_location {
        eprintln!("  重定向：{location}");
    }
    if let Some(error) = &entry.error {
        eprintln!("  传输错误：{error}");
    }

    if !entry.request_headers.is_empty() {
        eprintln!("  请求标头：");
        for header in &entry.request_headers {
            eprintln!("    {}: {}", header.name, header.value);
        }
    }

    if !entry.response_headers.is_empty() {
        eprintln!("  响应标头：");
        for header in &entry.response_headers {
            eprintln!("    {}: {}", header.name, header.value);
        }
    }

    if let Some(body) = entry.response_body_preview.as_deref() {
        eprintln!("  响应预览：");
        for line in body.lines() {
            eprintln!("    {line}");
        }
    }
}

/// Resolve connection from profile, env vars, or CLI flags.
/// Priority: CLI flags > env vars > profile.
fn resolve_connection(cli: &Cli) -> Result<SapConnection> {
    // If a profile is specified, load it as the base
    let (profile_conn, profile_name) = if let Some(ref name) = cli.profile {
        let (cfg, _) = config::load_config().context("加载配置文件失败")?;
        let profile = cfg.connections.get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "找不到连接配置 '{}'。运行 'sap-odata profile list' 查看可用配置。",
                name
            )
        })?;
        if profile.environment.as_deref() == Some("PRD") {
            eprintln!("  \u{26a0} '{name}' 是生产环境（PRD）系统");
        }
        let conn = config::resolve_connection(name, profile)
            .with_context(|| format!("解析连接配置 '{name}' 失败"))?;
        (Some(conn), Some(name.clone()))
    } else {
        (None, None)
    };

    // Build final connection, CLI/env overriding profile
    let base_url = cli
        .url
        .clone()
        .or_else(|| profile_conn.as_ref().map(|c| c.base_url.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "未指定 SAP URL。请使用 --url、SAP_BASE_URL 环境变量或 --profile。{}",
                if profile_name.is_none() {
                    " 运行 'sap-odata profile list' 查看已保存的连接配置。"
                } else {
                    ""
                }
            )
        })?;

    let client = cli
        .client
        .clone()
        .or_else(|| profile_conn.as_ref().map(|c| c.client.clone()))
        .unwrap_or_else(|| "100".to_string());

    let language = cli
        .language
        .clone()
        .or_else(|| profile_conn.as_ref().map(|c| c.language.clone()))
        .unwrap_or_else(|| "EN".to_string());

    // Check if profile uses a non-password auth mode
    let uses_non_basic_auth = profile_conn
        .as_ref()
        .is_some_and(|c| matches!(&c.auth, AuthConfig::Sso | AuthConfig::Browser));

    let insecure_tls = profile_conn.as_ref().is_some_and(|c| c.insecure_tls);

    if uses_non_basic_auth && cli.user.is_none() && cli.password.is_none() {
        let auth = profile_conn
            .as_ref()
            .map(|c| c.auth.clone())
            .unwrap_or(AuthConfig::Sso);
        let sso_delegate = profile_conn.as_ref().is_some_and(|c| c.sso_delegate);
        return Ok(SapConnection {
            base_url,
            client,
            language,
            auth,
            insecure_tls,
            sso_delegate,
        });
    }

    let username = cli
        .user
        .clone()
        .or_else(|| {
            profile_conn.as_ref().and_then(|c| match &c.auth {
                AuthConfig::Basic { username, .. } => Some(username.clone()),
                AuthConfig::Sso | AuthConfig::Browser => None,
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!("未指定用户名。请使用 --user、SAP_USER 环境变量、--profile 或 --sso。")
        })?;

    let password = cli
        .password
        .clone()
        .or_else(|| {
            profile_conn.as_ref().and_then(|c| match &c.auth {
                AuthConfig::Basic { password, .. } => Some(password.clone()),
                AuthConfig::Sso | AuthConfig::Browser => None,
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "未指定密码。请使用 --password、SAP_PASSWORD 环境变量、--profile 或 --sso。"
            )
        })?;

    Ok(SapConnection {
        base_url,
        client,
        language,
        auth: AuthConfig::Basic { username, password },
        insecure_tls,
        sso_delegate: false,
    })
}

/// Resolve the -s flag: alias → catalog lookup → raw path.
/// Returns None if no -s was given (OK for services/profile commands).
async fn resolve_service_path(cli: &Cli, sap_client: &SapClient) -> Result<Option<String>> {
    let raw = match &cli.service {
        Some(s) => s.clone(),
        None => return Ok(None),
    };

    // If it looks like an SAP OData service path (`/sap/...`), use it verbatim.
    // Anything else rooted at `/` — notably SAP namespace-prefixed catalog
    // names like `/NAMESPACE/SERVICE_NAME` — still needs catalog resolution.
    if raw.starts_with("/sap/") {
        return Ok(Some(raw));
    }

    // Try profile aliases first
    if let Some(ref profile_name) = cli.profile {
        let (cfg, _) = config::load_config().context("加载配置文件失败")?;
        if let Some(profile) = cfg.connections.get(profile_name) {
            // Case-insensitive alias lookup
            let alias_lower = raw.to_lowercase();
            for (alias_name, alias_path) in &profile.aliases {
                if alias_name.to_lowercase() == alias_lower {
                    eprintln!("  已解析别名 '{}' -> {}", raw, alias_path);
                    return Ok(Some(alias_path.clone()));
                }
            }
        }
    }

    // Try catalog resolution (searches V2 + V4 by technical name).
    // Banner goes to stderr so `metadata > out.xml` and similar
    // redirects capture only the data on stdout.
    eprintln!("  正在解析服务 '{}'...", raw);
    match sap_odata_core::catalog::resolve_service_by_name(sap_client, &raw).await {
        Ok(path) => {
            eprintln!("  已解析服务 '{}' -> {}", raw, path);
            Ok(Some(path))
        }
        Err(e) => Err(anyhow::anyhow!(
            "无法解析服务 '{}'：{}。请使用完整服务路径，或添加服务别名。",
            raw,
            e
        )),
    }
}

// ── Alias commands ──

fn handle_alias_command(
    action: &AliasAction,
    profile_name: Option<&str>,
    json: bool,
) -> Result<()> {
    let profile_name =
        profile_name.ok_or_else(|| anyhow::anyhow!("别名命令需要指定 --profile / -p"))?;

    match action {
        AliasAction::List => cmd_alias_list(profile_name, json),
        AliasAction::Add { name, path } => cmd_alias_add(profile_name, name, path),
        AliasAction::Remove { name } => cmd_alias_remove(profile_name, name),
    }
}

fn cmd_alias_list(profile_name: &str, json: bool) -> Result<()> {
    let (cfg, _) = config::load_config().context("加载配置文件失败")?;
    let profile = cfg
        .connections
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("找不到连接配置 '{}'", profile_name))?;

    if json {
        let entries: Vec<_> = profile
            .aliases
            .iter()
            .map(|(name, path)| serde_json::json!({ "name": name, "path": path }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if profile.aliases.is_empty() {
        println!(
            "连接配置 '{}' 尚无服务别名。使用 'sap-odata -p {} alias add <名称> <路径>' 添加。",
            profile_name, profile_name
        );
        return Ok(());
    }

    let mut table = Table::new();
    table.load_style(UTF8_FULL);
    table.set_header(vec![
        Cell::new("别名").fg(Color::DarkCyan),
        Cell::new("服务路径").fg(Color::DarkCyan),
    ]);

    for (name, path) in &profile.aliases {
        table.add_row(vec![Cell::new(name), Cell::new(path)]);
    }

    println!("{table}");
    Ok(())
}

fn cmd_alias_add(profile_name: &str, alias_name: &str, service_path: &str) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;
    let profile = cfg
        .connections
        .get_mut(profile_name)
        .ok_or_else(|| anyhow::anyhow!("找不到连接配置 '{}'", profile_name))?;

    profile
        .aliases
        .insert(alias_name.to_string(), service_path.to_string());
    config::save_connections(&cfg, &config_dir.path)?;
    println!(
        "  已为连接配置 '{}' 保存别名 '{}' -> {}。",
        profile_name, alias_name, service_path
    );
    Ok(())
}

fn cmd_alias_remove(profile_name: &str, alias_name: &str) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;
    let profile = cfg
        .connections
        .get_mut(profile_name)
        .ok_or_else(|| anyhow::anyhow!("找不到连接配置 '{}'", profile_name))?;

    if profile.aliases.remove(alias_name).is_none() {
        anyhow::bail!("连接配置 '{}' 中找不到别名 '{}'", profile_name, alias_name);
    }

    config::save_connections(&cfg, &config_dir.path)?;
    println!(
        "  已从连接配置 '{}' 中删除别名 '{}'。",
        profile_name, alias_name
    );
    Ok(())
}

// ── Profile commands ──

// ══════════════════════════════════════════════════════════════
// ── SETUP WIZARD ──
// ══════════════════════════════════════════════════════════════

async fn cmd_setup_wizard() -> Result<()> {
    use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};

    let theme = ColorfulTheme::default();

    println!();
    println!("SAP OData 浏览器——连接配置向导");
    println!("─────────────────────────────────────────");
    println!();

    // Profile name
    let name: String = Input::with_theme(&theme)
        .with_prompt("连接配置名称（例如 DEV、QAS、PRD）")
        .validate_with(|input: &String| -> std::result::Result<(), &str> {
            if input.trim().is_empty() {
                Err("名称不能为空")
            } else if input.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\') {
                Err("名称不能包含空白字符或斜杠")
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    let name = name.trim().to_string();

    // Global name-uniqueness check (step-7 hardening). Run BEFORE any
    // keyring writes / further prompts so we don't leave an orphan
    // credential entry under a name we're about to reject. Same shared
    // helper as `cmd_profile_add` and Tauri `add_profile`.
    {
        let (cfg_for_check, _) = config::load_config().context("读取配置文件以检查名称失败")?;
        if let Err(msg) = offline::check_connected_profile_name_available(&name, &cfg_for_check) {
            anyhow::bail!("{msg}");
        }
    }

    // Base URL
    let url: String = Input::with_theme(&theme)
        .with_prompt("SAP 基础 URL（例如 https://sap.example.com 或 https://host:44300）")
        .validate_with(|input: &String| -> std::result::Result<(), &str> {
            if !input.starts_with("http://") && !input.starts_with("https://") {
                Err("URL 必须以 http:// 或 https:// 开头")
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    let url = url.trim().trim_end_matches('/').to_string();

    // Client
    let client: String = Input::with_theme(&theme)
        .with_prompt("SAP 客户端编号")
        .default("100".to_string())
        .interact_text()?;

    // Language
    let language: String = Input::with_theme(&theme)
        .with_prompt("语言代码")
        .default("EN".to_string())
        .interact_text()?;

    // Auth method
    let auth_modes = &[
        "Basic（用户名 + 密码）",
        "Windows SSO（Kerberos / SPNEGO）",
        "浏览器 SSO（Azure AD / SAP IAS / SAML）",
    ];
    let auth_idx = Select::with_theme(&theme)
        .with_prompt("认证方式")
        .items(auth_modes)
        .default(0)
        .interact()?;

    let (username, password, sso, browser_sso) = match auth_idx {
        0 => {
            let user: String = Input::with_theme(&theme)
                .with_prompt("用户名")
                .interact_text()?;
            let pass = Password::with_theme(&theme)
                .with_prompt("密码")
                .interact()?;
            (user.trim().to_string(), pass, false, false)
        }
        1 => (String::new(), String::new(), true, false),
        2 => {
            println!();
            println!("  Browser SSO 需要先在桌面应用或 VS Code 扩展中登录一次。");
            println!("  登录后，此 CLI 配置也可以使用——");
            println!("  会话 Cookie 将安全地保存在操作系统凭据库中。");
            println!();
            (String::new(), String::new(), false, true)
        }
        _ => unreachable!(),
    };

    let env_choices = ["（无）", "DEV", "QAS", "PRD"];
    let env_idx = Select::with_theme(&theme)
        .with_prompt("系统环境（用于标记生产系统）")
        .items(env_choices)
        .default(0)
        .interact()?;
    let environment = (env_idx > 0).then(|| env_choices[env_idx].to_string());

    // Build profile — preserve existing aliases if updating
    let existing_aliases = config::load_config()
        .ok()
        .and_then(|(cfg, _)| cfg.connections.get(&name).map(|p| p.aliases.clone()))
        .unwrap_or_default();

    let profile = ConnectionProfile {
        base_url: url.clone(),
        client: client.clone(),
        language: language.clone(),
        username: username.clone(),
        password: None, // goes to keyring
        sso,
        browser_sso,
        insecure_tls: false,
        sso_delegate: false,
        aliases: existing_aliases,
        environment,
    };

    // Store password in keyring if Basic auth
    if auth_idx == 0 {
        match config::set_password_in_keyring(&name, &username, &password) {
            Ok(()) => println!("  密码已保存到操作系统凭据库。"),
            Err(e) => {
                println!("  警告：无法将密码保存到操作系统凭据库：{e}");
                println!("  （请重新运行向导，或设置 SAP_PASSWORD 环境变量以使用此配置。）");
            }
        }
    }

    // Save config
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;

    // If the connection fingerprint changed (url/client/language), discard any
    // persisted Browser SSO session so we don't replay cookies to a different target.
    let old_profile = cfg.connections.get(&name).cloned();
    match config::clear_session_if_connection_changed(
        &name,
        old_profile.as_ref(),
        &url,
        &client,
        &language,
    ) {
        Ok(true) => println!("  连接信息已更改，已清除过期的 Browser SSO 会话。"),
        Ok(false) => {}
        Err(e) => println!(
            "  警告：连接信息已更改，但无法清除过期会话（{e}）。\
             可能需要重新登录；可尝试运行 'sap-odata signout {name}'。"
        ),
    }

    cfg.connections.insert(name.clone(), profile);
    let save_path = if config_dir.path.as_os_str().is_empty() {
        let dir = config::get_or_create_config_dir()?;
        config::save_connections(&cfg, &dir.path)?
    } else {
        config::save_connections(&cfg, &config_dir.path)?
    };

    println!();
    println!("  连接配置 '{}' 已保存至 {}", name, save_path.display());

    // Offer to test the connection (except for Browser SSO which needs a webview)
    if !browser_sso {
        let do_test = Confirm::with_theme(&theme)
            .with_prompt("现在测试连接吗？")
            .default(true)
            .interact()?;
        if do_test {
            println!("  正在测试连接...");
            let conn = SapConnection {
                base_url: url,
                client,
                language,
                auth: if sso {
                    AuthConfig::Sso
                } else {
                    AuthConfig::Basic { username, password }
                },
                insecure_tls: false,
                sso_delegate: false,
            };
            match SapClient::new(conn) {
                Ok(client) => match client
                    .ensure_session("/sap/opu/odata/IWFND/CATALOGSERVICE;v=2")
                    .await
                {
                    Ok(()) => println!("  连接成功。"),
                    Err(e) => println!("  连接测试失败：{e}"),
                },
                Err(e) => println!("  无法创建客户端：{e}"),
            }
        }
    } else {
        println!();
        println!("  下一步：打开桌面应用，选择此配置并点击“登录”。");
        println!("  登录后即可在此终端中使用 CLI 命令，例如：");
        println!("    sap-odata -p {name} services");
        println!("  之后即可在此终端中使用。");
    }

    println!();
    Ok(())
}

// ══════════════════════════════════════════════════════════════
// ── SIGNOUT ──
// ══════════════════════════════════════════════════════════════

fn cmd_signout(name: &str) -> Result<()> {
    sap_odata_core::session::clear(name)
        .with_context(|| format!("清除连接配置 '{name}' 的会话失败"))?;
    println!("  已清除连接配置 '{name}' 保存的 Browser SSO 会话。");
    Ok(())
}

async fn handle_profile_command(action: &ProfileAction, json: bool) -> Result<()> {
    match action {
        ProfileAction::List => cmd_profile_list(json),
        ProfileAction::Add {
            name,
            url,
            client,
            language,
            user,
            password,
            sso,
            sso_delegate,
            plaintext,
            portable,
            environment,
        } => cmd_profile_add(
            name,
            url,
            client,
            language,
            user,
            password,
            *sso,
            *sso_delegate,
            *plaintext,
            *portable,
            environment.as_deref(),
        ),
        ProfileAction::Remove { name } => cmd_profile_remove(name),
        ProfileAction::Test { name } => cmd_profile_test(name).await,
        ProfileAction::Where => cmd_profile_where(json),
    }
}

/// Where a profile's credential comes from, as (JSON code, table label).
/// Never reads more than "is there a password" — the value is dropped.
fn profile_password_source(
    name: &str,
    profile: &config::ConnectionProfile,
) -> (&'static str, &'static str) {
    // Order matches Tauri's get_profiles: browser_sso and sso both have
    // empty usernames and no Basic password, so they must short-circuit
    // before the keyring lookup — otherwise Entry::new(target, "") can
    // surface a misleading "keyring error" on a profile that legitimately
    // has no password at all.
    if profile.browser_sso {
        ("browser_sso", "浏览器 SSO")
    } else if profile.sso {
        ("sso", "SSO（Windows）")
    } else if profile.password.is_some() {
        ("plaintext", "配置文件（明文）")
    } else {
        match config::try_get_password_from_keyring(name, &profile.username) {
            Ok(Some(_)) => ("keyring", "操作系统凭据库"),
            Ok(None) => ("not_set", "未设置"),
            Err(config::KeyringReadError::Locked(_)) => ("keyring_locked", "凭据库已锁定"),
            Err(config::KeyringReadError::Corrupt(_)) => ("keyring_corrupt", "凭据已损坏"),
            Err(config::KeyringReadError::Backend(_)) => ("keyring_error", "凭据库错误"),
        }
    }
}

fn cmd_profile_list(json: bool) -> Result<()> {
    let (cfg, config_dir) = config::load_config().context("加载配置文件失败")?;

    if json {
        let entries: Vec<_> = cfg
            .connections
            .iter()
            .map(|(name, profile)| {
                let auth = if profile.browser_sso {
                    "browser_sso"
                } else if profile.sso {
                    "sso"
                } else {
                    "basic"
                };
                serde_json::json!({
                    "name": name,
                    "base_url": profile.base_url,
                    "client": profile.client,
                    "language": profile.language,
                    "auth": auth,
                    "username": profile.username,
                    "password_source": profile_password_source(name, profile).0,
                    "insecure_tls": profile.insecure_tls,
                    "environment": profile.environment,
                    "aliases": profile.aliases,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if cfg.connections.is_empty() {
        println!("尚未保存连接配置。使用 'sap-odata profile add' 添加配置。");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_style(UTF8_FULL);
    table.set_header(vec![
        Cell::new("配置").fg(Color::DarkCyan),
        Cell::new("环境").fg(Color::DarkCyan),
        Cell::new("地址").fg(Color::DarkCyan),
        Cell::new("客户端").fg(Color::DarkCyan),
        Cell::new("语言").fg(Color::DarkCyan),
        Cell::new("用户").fg(Color::DarkCyan),
        Cell::new("密码").fg(Color::DarkCyan),
    ]);

    for (name, profile) in &cfg.connections {
        let auth_info = profile_password_source(name, profile).1;

        let env = profile.environment.as_deref().unwrap_or("");
        let env_cell = match env {
            "PRD" => Cell::new(env).fg(Color::Red),
            "QAS" => Cell::new(env).fg(Color::Yellow),
            _ => Cell::new(env),
        };
        table.add_row(vec![
            Cell::new(name),
            env_cell,
            Cell::new(&profile.base_url),
            Cell::new(&profile.client),
            Cell::new(&profile.language),
            Cell::new(if profile.browser_sso {
                "（浏览器 SSO）"
            } else if profile.sso {
                "（SSO）"
            } else {
                &profile.username
            }),
            Cell::new(auth_info),
        ]);
    }

    println!("{table}");
    let mode = if config_dir.is_portable {
        "便携式"
    } else {
        "用户"
    };
    println!("\n  配置目录：{}（{}）\n", config_dir.path.display(), mode);

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_profile_add(
    name: &str,
    url: &str,
    client: &str,
    language: &str,
    user: &str,
    password: &str,
    sso: bool,
    sso_delegate: bool,
    plaintext: bool,
    portable: bool,
    environment: Option<&str>,
) -> Result<()> {
    // Validate before touching anything.
    let environment = environment
        .map(config::normalize_environment)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;

    // Global name-uniqueness check (step 7's connected-side enforcement).
    // Shared core helper so this path and `cmd_setup_wizard` and the
    // Tauri `add_profile` all enforce identical semantics.
    if let Err(msg) = offline::check_connected_profile_name_available(name, &cfg) {
        anyhow::bail!("{msg}");
    }

    let existing_aliases = cfg
        .connections
        .get(name)
        .map(|p| p.aliases.clone())
        .unwrap_or_default();

    // If the connection fingerprint changed (url/client/language), discard any
    // persisted Browser SSO session so we don't replay cookies to a different target.
    let old_profile = cfg.connections.get(name).cloned();
    match config::clear_session_if_connection_changed(
        name,
        old_profile.as_ref(),
        url,
        client,
        language,
    ) {
        Ok(true) => println!("  连接信息已更改，已清除过期的 Browser SSO 会话。"),
        Ok(false) => {}
        Err(e) => println!(
            "  警告：连接信息已更改，但无法清除过期会话（{e}）。\
             可能需要重新登录；可尝试运行 'sap-odata signout {name}'。"
        ),
    }

    // `profile add` has no Browser SSO flag: re-running it on a Browser SSO
    // profile without credentials (e.g. to change the URL) keeps that mode
    // instead of silently converting the profile to Basic.
    let keep_browser = !sso
        && user.is_empty()
        && password.is_empty()
        && old_profile.as_ref().is_some_and(|p| p.browser_sso);
    let is_basic = !sso && !keep_browser;
    let carry =
        config::profile_carry_over(old_profile.as_ref(), is_basic, user, !password.is_empty());
    let same_basic_user = old_profile
        .as_ref()
        .is_some_and(|p| !p.sso && !p.browser_sso && p.username == user);

    let profile = ConnectionProfile {
        base_url: url.to_string(),
        client: client.to_string(),
        language: language.to_string(),
        username: if keep_browser {
            String::new()
        } else {
            user.to_string()
        },
        password: if is_basic && plaintext && !password.is_empty() {
            Some(password.to_string())
        } else {
            carry.plaintext_password.clone()
        },
        sso,
        browser_sso: keep_browser,
        insecure_tls: carry.insecure_tls,
        sso_delegate: sso && sso_delegate,
        aliases: existing_aliases,
        // `--env none` clears it; omitted keeps the current value.
        environment: match environment {
            Some(env) => env,
            None => old_profile.as_ref().and_then(|p| p.environment.clone()),
        },
    };

    // Store a given password in the keyring unless plaintext requested.
    // Fail closed: do NOT silently fall back to plaintext on keyring failure —
    // that's exactly the kind of surprise an enterprise security review rejects.
    // The user has to explicitly opt in with --plaintext.
    let password_note = if !is_basic {
        None
    } else if !password.is_empty() && !plaintext {
        if let Err(e) = config::set_password_in_keyring(name, user, password) {
            anyhow::bail!(
                "无法将密码保存到操作系统凭据库：{e}
                 请使用 --plaintext 将密码改为保存到配置文件（不推荐），或修复凭据库后重试。"
            );
        }
        Some("密码已保存到操作系统凭据库。")
    } else if !password.is_empty() {
        Some("密码已以明文形式保存到配置文件。")
    } else if carry.plaintext_password.is_some() {
        Some("密码未更改（保留配置文件中的密码）。")
    } else if same_basic_user {
        Some("密码未更改（保留凭据库中的现有密码）。")
    } else {
        Some("尚未保存密码——请使用 --password，或运行命令时设置 SAP_PASSWORD。")
    };

    cfg.connections.insert(name.to_string(), profile);

    let save_path = if portable {
        config::init_portable_config(&cfg)?
    } else if config_dir.path.as_os_str().is_empty() {
        let dir = config::get_or_create_config_dir()?;
        config::save_connections(&cfg, &dir.path)?
    } else {
        config::save_connections(&cfg, &config_dir.path)?
    };

    if let Some(note) = password_note {
        println!("  {note}");
    }
    // After the save, so a failed save never loses the old credential.
    if let Some(old_user) = carry.orphaned_keyring_user.as_deref() {
        match config::delete_password_from_keyring(name, old_user) {
            Ok(()) => println!("  已删除旧的凭据库条目（用户：{old_user}）。"),
            Err(e) => println!("  警告：无法删除旧的凭据库条目（用户：{old_user}）：{e}"),
        }
    }
    let mode = if sso {
        "（SSO）"
    } else if keep_browser {
        "（Browser SSO）"
    } else {
        ""
    };
    println!(
        "  连接配置 '{}' {mode}已保存至 {}",
        name,
        save_path.display()
    );
    Ok(())
}

fn cmd_profile_remove(name: &str) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;

    let profile = cfg.connections.remove(name);
    if profile.is_none() {
        anyhow::bail!("找不到连接配置 '{}'", name);
    }

    let profile = profile.unwrap();

    // Remove password from keyring (basic auth). If the keyring is unavailable
    // the credential stays stored — warn the user so they can clean it up.
    let mut warnings: Vec<String> = Vec::new();
    if !profile.username.is_empty()
        && let Err(e) = config::delete_password_from_keyring(name, &profile.username)
    {
        warnings.push(format!("密码（用户：{}）——{e}", profile.username));
    }
    // Remove any persisted Browser SSO session — critical so that recreating
    // a profile with the same name doesn't resurrect the old session.
    if let Err(e) = sap_odata_core::session::clear(name) {
        warnings.push(format!("Browser SSO 会话——{e}"));
    }

    config::save_connections(&cfg, &config_dir.path)?;

    if warnings.is_empty() {
        println!("  已删除连接配置 '{}'（凭据和会话均已清除）。", name);
    } else {
        println!(
            "  已从配置文件中删除连接配置 '{}'，但以下项目未能清除：",
            name
        );
        for w in &warnings {
            println!("    - {w}");
        }
        println!("  你可能需要手动从操作系统凭据库中删除这些项目。");
    }
    Ok(())
}

async fn cmd_profile_test(name: &str) -> Result<()> {
    let (cfg, _) = config::load_config().context("加载配置文件失败")?;
    let profile = cfg
        .connections
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("找不到连接配置 '{}'", name))?;

    let connection = config::resolve_connection(name, profile)
        .with_context(|| format!("解析连接配置 '{name}' 失败"))?;

    println!(
        "  正在测试与 {} 的连接（客户端 {}）...",
        connection.base_url, connection.client
    );

    let client = SapClient::new(connection).context("创建 SAP 客户端失败")?;

    // Try to establish a session
    client
        .ensure_session("/sap/opu/odata/IWFND/CATALOGSERVICE;v=2")
        .await
        .context("连接测试失败")?;

    println!("  连接成功！");
    Ok(())
}

fn cmd_profile_where(json: bool) -> Result<()> {
    if json {
        let dir = match config::find_config_dir() {
            Some(dir) => dir,
            None => config::get_or_create_config_dir()?,
        };
        let payload = serde_json::json!({
            "path": dir.path,
            "portable": dir.is_portable,
            "config_file_exists": dir.path.join(config::CONFIG_FILENAME).is_file(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    match config::find_config_dir() {
        Some(dir) => {
            let mode = if dir.is_portable {
                "便携式"
            } else {
                "用户"
            };
            println!("  配置目录：{}（{}）", dir.path.display(), mode);
        }
        None => {
            // Show where it would be created
            let dir = config::get_or_create_config_dir()?;
            let mode = if dir.is_portable {
                "便携式"
            } else {
                "用户"
            };
            println!(
                "  配置目录：{}（{}，首次使用时创建）",
                dir.path.display(),
                mode
            );
        }
    }
    Ok(())
}

// ── Query builder ──

/// Combine the raw `--filter` expression with a `--in PROP V1 V2 ...`
/// shortcut. Returns the merged $filter expression, or the original
/// filter / None when `--in` is empty.
///
/// `--in` builds `(PROP eq 'V1' or PROP eq 'V2' or ...)` with each value
/// quoted as an OData string literal (apostrophes doubled per spec —
/// `O'Brien` → `'O''Brien'`). Combined with an explicit `--filter` by
/// wrapping both in parens and joining with `and`, so operator
/// precedence stays intact. String literals only — numeric/date typing
/// would need property metadata.
fn combine_filter_with_in(filter: Option<String>, in_values: &[String]) -> Result<Option<String>> {
    if in_values.is_empty() {
        return Ok(filter);
    }
    // clap's `num_args = 2..` already enforces this, but guard for
    // defensive callers (and to keep the error specific).
    if in_values.len() < 2 {
        anyhow::bail!(
            "--in 需要一个属性名称，后跟一个或多个值（当前收到 {} 个参数）",
            in_values.len()
        );
    }
    let prop = &in_values[0];
    let in_expr = in_values[1..]
        .iter()
        .map(|v| format!("{prop} eq '{}'", v.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(" or ");
    let in_expr = format!("({in_expr})");

    Ok(Some(match filter {
        Some(f) => format!("({f}) and {in_expr}"),
        None => in_expr,
    }))
}

/// OData version implied by an SAP service path (`/sap/opu/odata4/…` is
/// V4). `build` sends no request, so it can't read `$metadata`; the path
/// convention decides version-specific syntax such as `$count=true` (V4)
/// vs `$inlinecount=allpages` (V2).
fn version_from_service_path(path: &str) -> ODataVersion {
    if path.contains("/odata4/") {
        ODataVersion::V4
    } else {
        ODataVersion::V2
    }
}

#[allow(clippy::too_many_arguments)]
fn build_query(
    entity_set: &str,
    select: Option<String>,
    filter: Option<String>,
    expand: Option<String>,
    orderby: Option<String>,
    top: Option<u32>,
    skip: Option<u32>,
    key: Option<String>,
    count: bool,
    version: Option<ODataVersion>,
) -> ODataQuery {
    let mut q = ODataQuery::new(entity_set).format("json");
    if let Some(v) = version {
        q = q.version(v);
    }

    if let Some(s) = select {
        let fields: Vec<&str> = s.split(',').map(str::trim).collect();
        q = q.select(&fields);
    }
    if let Some(f) = filter {
        q = q.filter(&f);
    }
    if let Some(e) = expand {
        let navs: Vec<&str> = e.split(',').map(str::trim).collect();
        q = q.expand(&navs);
    }
    if let Some(o) = orderby {
        let clauses: Vec<&str> = o.split(',').map(str::trim).collect();
        q = q.orderby(&clauses);
    }
    if let Some(t) = top {
        q = q.top(t);
    }
    if let Some(s) = skip {
        q = q.skip(s);
    }
    if let Some(k) = key {
        q = q.key(&k);
    }
    if count {
        q = q.count();
    }
    q
}

// ── Service commands ──

async fn cmd_services(
    client: &SapClient,
    search: Option<&str>,
    v2_only: bool,
    v4_only: bool,
    top: Option<usize>,
    json: bool,
) -> Result<()> {
    let result = sap_odata_core::catalog::fetch_service_catalog(client)
        .await
        .context("获取服务目录失败")?;

    // Nothing listed and at least one catalog failed (401, expired
    // session, DNS, 403 …): that is an error, not "no services". Same
    // rule as the desktop app. A partial result (e.g. V4 catalog not
    // published) still lists what was found, with warnings on stderr.
    if result.entries.is_empty() && !result.warnings.is_empty() {
        let warnings = result
            .warnings
            .iter()
            .map(|warning| localize_catalog_warning(warning))
            .collect::<Vec<_>>()
            .join("；");
        anyhow::bail!("无法获取服务目录：{}", warnings);
    }

    for w in &result.warnings {
        eprintln!("  警告：{}", localize_catalog_warning(w));
    }

    let mut filtered: Vec<_> = result
        .entries
        .iter()
        .filter(|e| {
            if v2_only && e.is_v4 {
                return false;
            }
            if v4_only && !e.is_v4 {
                return false;
            }
            if let Some(s) = search {
                return e.matches(s);
            }
            true
        })
        .collect();

    let total = filtered.len();
    if let Some(n) = top {
        filtered.truncate(n);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("没有找到服务。");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_style(UTF8_FULL);
    table.set_header(vec![
        Cell::new("#").fg(Color::DarkCyan),
        Cell::new("版本").fg(Color::DarkCyan),
        Cell::new("技术名称").fg(Color::DarkCyan),
        Cell::new("标题").fg(Color::DarkCyan),
        Cell::new("服务 URL").fg(Color::DarkCyan),
    ]);

    for (i, entry) in filtered.iter().enumerate() {
        table.add_row(vec![
            Cell::new(i + 1),
            Cell::new(entry.version_label()),
            Cell::new(&entry.technical_name),
            Cell::new(&entry.title),
            Cell::new(&entry.service_url),
        ]);
    }

    println!("{table}");
    if filtered.len() < total {
        println!("\n  显示 {} / {} 个服务\n", filtered.len(), total);
    } else {
        println!("\n  共找到 {} 个服务\n", total);
    }

    Ok(())
}

fn localize_catalog_warning(warning: &str) -> String {
    for (english, chinese) in [
        ("V2 catalog:", "V2 服务目录："),
        ("V4 catalog:", "V4 服务目录："),
    ] {
        if let Some(detail) = warning.strip_prefix(english) {
            return format!("{chinese}{}", detail.trim_start());
        }
    }
    warning.to_string()
}

/// Where a metadata-only command reads `$metadata` from: the live
/// service, or an offline bucket's cached EDMX (`-p "<bucket>"`).
enum MetaSource<'a> {
    Live {
        client: &'a SapClient,
        service: &'a str,
    },
    Offline {
        xml: String,
        /// Original service path, for saves from a live system.
        service_path: Option<String>,
    },
}

impl MetaSource<'_> {
    fn service_path(&self) -> Option<&str> {
        match self {
            MetaSource::Live { service, .. } => Some(service),
            MetaSource::Offline { service_path, .. } => service_path.as_deref(),
        }
    }

    async fn xml(&self) -> Result<String> {
        match self {
            MetaSource::Live { client, service } => client
                .fetch_metadata_xml(service)
                .await
                .context("获取元数据失败"),
            MetaSource::Offline { xml, .. } => Ok(xml.clone()),
        }
    }

    async fn parsed(&self) -> Result<sap_odata_core::metadata::ServiceMetadata> {
        match self {
            MetaSource::Live { client, service } => client
                .fetch_metadata(service)
                .await
                .context("获取元数据失败"),
            MetaSource::Offline { xml, .. } => {
                sap_odata_core::metadata::parse_metadata(xml).context("解析缓存的 EDMX 失败")
            }
        }
    }
}

/// `-p` names an offline bucket. The metadata commands work against the
/// cached EDMX; anything that would talk to SAP is refused up front.
async fn run_offline_bucket_command(
    cli: &Cli,
    cfg: &config::ConfigFile,
    config_dir: &std::path::Path,
    bucket: &str,
) -> Result<()> {
    if let Commands::Services { .. } = &cli.command {
        return render_offline_services(cfg, bucket, cli.json);
    }
    let metadata_only = matches!(
        &cli.command,
        Commands::Entities
            | Commands::Describe { .. }
            | Commands::Functions
            | Commands::Metadata
            | Commands::Annotations { .. }
            | Commands::Lint { .. }
    );
    if !metadata_only {
        anyhow::bail!(
            "'{bucket}' 是离线配置（缓存了 $metadata），无法连接 SAP 系统执行查询。\
             以下命令支持离线运行：services、entities、describe、functions、annotations、lint、metadata。"
        );
    }
    let ident = cli.service.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "必须指定 --service / -s：请使用 `sap-odata offline list --profile \"{bucket}\"` 中的服务 ID 或标签"
        )
    })?;
    let service_id = resolve_offline_service(cfg, bucket, ident)?;
    let xml = offline::read_offline_metadata(cfg, config_dir, bucket, &service_id)
        .context("读取缓存的 EDMX 失败")?;
    let service_path = cfg
        .offline_services
        .iter()
        .find(|s| s.profile == bucket && s.id == service_id)
        .and_then(|s| s.source_service_path.clone());
    let src = MetaSource::Offline { xml, service_path };
    match &cli.command {
        Commands::Entities => cmd_entities(&src, cli.json).await,
        Commands::Describe { entity_set } => cmd_describe(&src, entity_set, cli.json).await,
        Commands::Functions => cmd_functions(&src, cli.json).await,
        Commands::Metadata => cmd_metadata(&src).await,
        Commands::Annotations { namespace, filter } => {
            cmd_annotations(&src, cli.json, namespace.clone(), filter.clone()).await
        }
        Commands::Lint {
            entity,
            min_severity,
            lint_profile,
            fail_on,
        } => {
            cmd_lint(
                &src,
                cli.json,
                entity.clone(),
                min_severity.clone(),
                *lint_profile,
                *fail_on,
            )
            .await
        }
        _ => unreachable!("filtered above"),
    }
}

/// Offline service by id, original service path, or (unique,
/// case-insensitive) label — the label is what `offline list` shows first.
fn resolve_offline_service(cfg: &config::ConfigFile, bucket: &str, ident: &str) -> Result<String> {
    use sap_odata_core::offline::{MetadataSource, MetadataSourceError};
    match MetadataSource::resolve(bucket, ident, cfg) {
        Ok(MetadataSource::Offline { service_id, .. }) => return Ok(service_id),
        Ok(MetadataSource::Connected { .. }) => {
            anyhow::bail!("'{bucket}' 是连接配置，不是离线配置桶")
        }
        Err(MetadataSourceError::OfflineServiceNotFound { .. }) => {}
        Err(e) => return Err(e.into()),
    }
    let by_label: Vec<_> = cfg
        .offline_services
        .iter()
        .filter(|s| s.profile == bucket && s.label.eq_ignore_ascii_case(ident))
        .collect();
    match by_label.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => anyhow::bail!(
            "离线配置桶 '{bucket}' 中没有服务 '{ident}'。运行 `sap-odata offline list --profile \"{bucket}\"` 查看服务 ID 和标签。"
        ),
        _ => anyhow::bail!("'{ident}' 在 '{bucket}' 中匹配到多个服务，请使用服务 ID"),
    }
}

async fn cmd_entities(src: &MetaSource<'_>, json: bool) -> Result<()> {
    let meta = src.parsed().await?;

    if json {
        let sets: Vec<_> = meta
            .entity_sets
            .iter()
            .map(|es| {
                serde_json::json!({
                    "name": es.name,
                    "entity_type": es.entity_type,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&sets)?);
    } else {
        let mut table = Table::new();
        table.load_style(UTF8_FULL);
        table.set_header(vec![
            Cell::new("#").fg(Color::DarkCyan),
            Cell::new("实体集").fg(Color::DarkCyan),
            Cell::new("实体类型").fg(Color::DarkCyan),
            Cell::new("键").fg(Color::DarkCyan),
        ]);

        // Hide SAP V4 system entity sets from the table; --json (handled
        // above) still prints them. Re-number with `enumerate` after the
        // filter so the displayed index is 1..N over what's shown.
        for (i, es) in meta
            .entity_sets
            .iter()
            .filter(|es| !is_sap_system_field(&es.name))
            .enumerate()
        {
            let et = meta.entity_type_for_set(&es.name);
            let keys = et.map(|t| t.keys.join(", ")).unwrap_or_default();
            table.add_row(vec![
                Cell::new(i + 1),
                Cell::new(&es.name),
                Cell::new(&es.entity_type),
                Cell::new(keys),
            ]);
        }

        println!("{table}");
    }
    Ok(())
}

async fn cmd_describe(src: &MetaSource<'_>, entity_set: &str, json: bool) -> Result<()> {
    let meta = src.parsed().await?;

    let et = meta
        .entity_type_for_set(entity_set)
        .ok_or_else(|| anyhow::anyhow!("找不到实体集 '{entity_set}'"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&et)?);
        return Ok(());
    }

    println!("\n  实体类型：{}", et.name);
    println!("  键：{}\n", et.keys.join(", "));

    let mut prop_table = Table::new();
    prop_table.load_style(UTF8_FULL);
    prop_table.set_header(vec![
        Cell::new("属性").fg(Color::DarkCyan),
        Cell::new("类型").fg(Color::DarkCyan),
        Cell::new("键").fg(Color::DarkCyan),
        Cell::new("可空").fg(Color::DarkCyan),
        Cell::new("最大长度").fg(Color::DarkCyan),
        Cell::new("标签").fg(Color::DarkCyan),
    ]);

    // Hide SAP V4 system fields (`SAP__Messages`, `__OperationControl`,
    // ...) from the property table. --json output above keeps the full
    // entity type intact so scripted consumers can still see them.
    for p in et
        .properties
        .iter()
        .filter(|p| !is_sap_system_field(&p.name))
    {
        let is_key = if et.keys.contains(&p.name) { "*" } else { "" };
        prop_table.add_row(vec![
            Cell::new(&p.name),
            Cell::new(&p.edm_type),
            Cell::new(is_key),
            Cell::new(if p.nullable { "是" } else { "否" }),
            Cell::new(p.max_length.map(|l| l.to_string()).unwrap_or_default()),
            Cell::new(p.label.as_deref().unwrap_or("")),
        ]);
    }
    println!("{prop_table}");

    let nav_targets: Vec<_> = meta
        .nav_targets(et)
        .into_iter()
        .filter(|(name, _, _)| !is_sap_system_field(name))
        .collect();
    if !nav_targets.is_empty() {
        println!("\n  导航属性：\n");
        let mut nav_table = Table::new();
        nav_table.load_style(UTF8_FULL);
        nav_table.set_header(vec![
            Cell::new("导航属性").fg(Color::DarkCyan),
            Cell::new("目标类型").fg(Color::DarkCyan),
            Cell::new("多重性").fg(Color::DarkCyan),
        ]);
        for (name, target, mult) in &nav_targets {
            nav_table.add_row(vec![Cell::new(name), Cell::new(target), Cell::new(mult)]);
        }
        println!("{nav_table}");
    }

    println!("\n  查询示例：");
    // Skip SAP V4 system fields when picking example $select columns —
    // showing `SAP__Messages` in the example query would mislead the user.
    let select_fields: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| !is_sap_system_field(&p.name))
        .take(3)
        .map(|p| p.name.as_str())
        .collect();
    println!(
        "  sap-odata ... run {entity_set} --select {} --top 10\n",
        select_fields.join(",")
    );
    if let Some(service) = src.service_path() {
        println!(
            "  URL：{}/{}\n",
            service,
            ODataQuery::new(entity_set)
                .select(&select_fields)
                .top(10)
                .format("json")
                .build()
        );
    }

    Ok(())
}

async fn cmd_functions(src: &MetaSource<'_>, json: bool) -> Result<()> {
    let meta = src.parsed().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&meta.function_imports)?);
        return Ok(());
    }

    if meta.function_imports.is_empty() {
        println!("此服务中没有函数导入。");
        return Ok(());
    }

    {
        let mut table = Table::new();
        table.load_style(UTF8_FULL);
        table.set_header(vec![
            Cell::new("函数").fg(Color::DarkCyan),
            Cell::new("方法").fg(Color::DarkCyan),
            Cell::new("返回类型").fg(Color::DarkCyan),
            Cell::new("参数").fg(Color::DarkCyan),
        ]);

        for fi in &meta.function_imports {
            let params: Vec<String> = fi
                .parameters
                .iter()
                .map(|p| format!("{}: {}", p.name, p.edm_type))
                .collect();
            table.add_row(vec![
                Cell::new(&fi.name),
                Cell::new(&fi.http_method),
                Cell::new(fi.return_type.as_deref().unwrap_or("-")),
                Cell::new(params.join(", ")),
            ]);
        }
        println!("{table}");
    }
    Ok(())
}

async fn cmd_run(
    client: &SapClient,
    service: &str,
    query: &ODataQuery,
    json_output: bool,
) -> Result<()> {
    let data = client
        .query_json(service, query)
        .await
        .context("执行查询失败")?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    let results = extract_results(&data);

    match results {
        Some(serde_json::Value::Array(rows)) if !rows.is_empty() => {
            print_results_table(&rows);
        }
        Some(obj @ serde_json::Value::Object(_)) => {
            print_results_table(&[obj]);
        }
        _ => {
            println!("{}", serde_json::to_string_pretty(&data)?);
        }
    }

    Ok(())
}

/// `verify` — issue a small `$top` GET against every entity set in
/// the service and report OK / FAIL per set. Returns Err only on
/// fatal errors (metadata fetch failed); per-set failures are
/// reported in the table and surface via the exit code.
async fn cmd_verify(
    client: &SapClient,
    service: &str,
    quick: bool,
    top: u32,
    json: bool,
) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("获取元数据失败")?;
    let version = meta.version;

    // Skip SAP V4 framework sets (SAP__Messages, __OperationControl,
    // ...). Same rule cmd_entities uses for its table view — they're
    // runtime plumbing, not part of the data model under test.
    let sets: Vec<&str> = meta
        .entity_sets
        .iter()
        .map(|es| es.name.as_str())
        .filter(|name| !is_sap_system_field(name))
        .collect();

    if sets.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("此服务中没有可供验证的实体集。");
        }
        return Ok(());
    }

    let mut results: Vec<VerifyResult> = Vec::with_capacity(sets.len());
    for set in &sets {
        if quick {
            results.push(VerifyResult {
                set: (*set).to_string(),
                status: VerifyStatus::Ok,
                rows: None,
                note: Some("未探测（--quick）".to_string()),
            });
            continue;
        }
        let query = ODataQuery::new(set)
            .top(top)
            .format("json")
            .version(version);
        match client.query_json(service, &query).await {
            Ok(data) => {
                let rows = match extract_results(&data) {
                    Some(serde_json::Value::Array(arr)) => Some(arr.len() as u32),
                    Some(serde_json::Value::Object(_)) => Some(1),
                    _ => None,
                };
                results.push(VerifyResult {
                    set: (*set).to_string(),
                    status: VerifyStatus::Ok,
                    rows,
                    note: None,
                });
            }
            Err(e) => {
                let note = truncate_for_table(&e.to_string(), 120);
                results.push(VerifyResult {
                    set: (*set).to_string(),
                    status: VerifyStatus::Fail,
                    rows: None,
                    note: Some(note),
                });
            }
        }
    }

    let any_fail = results
        .iter()
        .any(|r| matches!(r.status, VerifyStatus::Fail));

    if json {
        let payload: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "set": r.set,
                    "status": match r.status {
                        VerifyStatus::Ok => "ok",
                        VerifyStatus::Fail => "fail",
                    },
                    "rows": r.rows,
                    "note": r.note,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        let mut table = Table::new();
        table.load_style(UTF8_FULL);
        table.set_header(vec![
            Cell::new("实体集").fg(Color::DarkCyan),
            Cell::new("状态").fg(Color::DarkCyan),
            Cell::new("行数").fg(Color::DarkCyan),
            Cell::new("说明").fg(Color::DarkCyan),
        ]);
        for r in &results {
            let status_cell = match r.status {
                VerifyStatus::Ok => Cell::new("成功").fg(Color::Green),
                VerifyStatus::Fail => Cell::new("失败").fg(Color::Red),
            };
            let rows_cell = match r.rows {
                Some(n) => Cell::new(n),
                None => Cell::new("-"),
            };
            table.add_row(vec![
                Cell::new(&r.set),
                status_cell,
                rows_cell,
                Cell::new(r.note.as_deref().unwrap_or("")),
            ]);
        }
        println!("{table}");

        let ok = results
            .iter()
            .filter(|r| matches!(r.status, VerifyStatus::Ok))
            .count();
        let fail = results.len() - ok;
        eprintln!(
            "\n  {} 成功 · {} 失败（已探测 {} 个实体集）",
            ok,
            fail,
            results.len()
        );
    }

    if any_fail {
        // Bail (rather than `process::exit`) so the caller's post-
        // command paths still run — most importantly `emit_http_trace`
        // under `--verbose`, which is exactly when failure diagnostics
        // are useful. The terse message is one line under the table
        // and doesn't compete with the per-row failure details.
        let fail = results
            .iter()
            .filter(|r| matches!(r.status, VerifyStatus::Fail))
            .count();
        anyhow::bail!("{fail} 个实体集验证失败");
    }
    Ok(())
}

/// Per-set outcome captured during `cmd_verify`. Kept inside the
/// fn module so the JSON/table renderers can share it; not exported.
struct VerifyResult {
    set: String,
    status: VerifyStatus,
    rows: Option<u32>,
    note: Option<String>,
}

enum VerifyStatus {
    Ok,
    Fail,
}

/// Trim a multi-line error message down to the first line and cap
/// at `max_len` chars so the verify table stays scannable.
fn truncate_for_table(s: &str, max_len: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.chars().count() <= max_len {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(max_len).collect();
        format!("{truncated}…")
    }
}

async fn cmd_metadata(src: &MetaSource<'_>) -> Result<()> {
    let xml = src.xml().await?;
    println!("{xml}");
    Ok(())
}

async fn cmd_annotations(
    src: &MetaSource<'_>,
    as_json: bool,
    namespace: Option<String>,
    filter: Option<String>,
) -> Result<()> {
    let meta = src.parsed().await?;

    // Optional filters applied in order: namespace (exact-ish match) +
    // free-text substring across term/target/value/qualifier.
    let ns_needle = namespace.as_deref().map(|s| s.to_ascii_lowercase());
    let text_needle = filter.as_deref().map(|s| s.to_ascii_lowercase());
    let filtered: Vec<_> = meta
        .annotations
        .into_iter()
        .filter(|a| {
            if let Some(ns) = &ns_needle
                && a.namespace.to_ascii_lowercase() != *ns
            {
                return false;
            }
            if let Some(n) = &text_needle {
                let hay = format!(
                    "{} {} {} {}",
                    a.term,
                    a.target,
                    a.value.as_deref().unwrap_or(""),
                    a.qualifier.as_deref().unwrap_or(""),
                )
                .to_ascii_lowercase();
                if !hay.contains(n) {
                    return false;
                }
            }
            true
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("没有符合筛选条件的注解。");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_style(UTF8_FULL).set_header(vec![
        Cell::new("命名空间").fg(Color::Cyan),
        Cell::new("术语").fg(Color::Cyan),
        Cell::new("目标").fg(Color::Cyan),
        Cell::new("值").fg(Color::Cyan),
        Cell::new("限定符").fg(Color::Cyan),
    ]);
    for a in &filtered {
        table.add_row(vec![
            Cell::new(&a.namespace),
            Cell::new(&a.term),
            Cell::new(&a.target),
            Cell::new(a.value.as_deref().unwrap_or("—")),
            Cell::new(a.qualifier.as_deref().unwrap_or("")),
        ]);
    }
    println!("{table}");
    println!("\n共显示 {} 条注解。", filtered.len());
    Ok(())
}

async fn cmd_lint(
    src: &MetaSource<'_>,
    as_json: bool,
    entity: Option<String>,
    min_severity: Option<String>,
    profile_override: Option<LintProfileArg>,
    fail_on: Option<FailOnArg>,
) -> Result<()> {
    let meta = src.parsed().await?;

    let min_sev = match min_severity.as_deref() {
        Some("pass") | None => None,
        Some("warn") => Some(LintSeverity::Warn),
        Some("miss") => Some(LintSeverity::Miss),
        Some(other) => {
            anyhow::bail!("未知的严重级别 '{other}'——有效值为 pass、warn 或 miss");
        }
    };
    let fail_on_sev: Option<LintSeverity> = fail_on.map(Into::into);

    // Resolve the scope: one entity (accept either type name or set
    // name), or every entity type in the schema.
    let targets: Vec<&str> = match entity.as_deref() {
        Some(name) => {
            if meta.find_entity_type(name).is_some() {
                vec![name]
            } else if let Some(et) = meta.entity_type_for_set(name) {
                vec![et.name.as_str()]
            } else {
                anyhow::bail!("找不到实体类型或实体集 '{name}'");
            }
        }
        None => meta
            .entity_types
            .iter()
            .map(|et| et.name.as_str())
            .collect(),
    };

    // Top-level profile fields are intentionally separate from the
    // per-finding banner: programmatic consumers (CI, agents) read the
    // structured fields, while the banner stays in `findings` so the
    // text-rendering path keeps its current shape.
    //
    // `detected_profile` is always the heuristic auto-detection — it
    // reflects what the linter would pick if no override were supplied.
    // `effective_profile` is what the rules actually ran against. Without
    // `--profile` they match. With `--profile`, callers can compare the
    // two to report "we auto-detected X but you forced Y" without losing
    // either signal.
    #[derive(serde::Serialize)]
    struct Report<'a> {
        entity: &'a str,
        detected_profile: lint::LintProfile,
        effective_profile: lint::LintProfile,
        findings: Vec<lint::LintFinding>,
    }
    let mut reports: Vec<Report> = Vec::new();
    let mut fail_on_tripped = false;
    for name in &targets {
        let et = meta
            .find_entity_type(name)
            .ok_or_else(|| anyhow::anyhow!("实体 '{name}' 不存在"))?;
        let detected_profile = lint::detect_profile(et);
        let effective_profile = profile_override.map(Into::into).unwrap_or(detected_profile);
        let mut findings = lint::evaluate_entity_type_with_profile(et, effective_profile);
        // Compute fail-on hit BEFORE the min-severity retain — fail-on
        // is an exit-code gate, not a display gate, and a user who runs
        // `--min-severity miss --fail-on warn` still wants warns to
        // count toward the exit code even if they're filtered out of
        // the table.
        if let Some(threshold) = fail_on_sev
            && findings
                .iter()
                .any(|f| f.code != "profile" && severity_at_least(f.severity, threshold))
        {
            fail_on_tripped = true;
        }
        if let Some(s) = min_sev {
            // Keep the "profile" banner regardless of severity so the
            // user still sees which profile was assumed when they ask
            // for only warns/misses. Every other Pass is filtered out
            // as intended.
            findings.retain(|f| f.code == "profile" || severity_at_least(f.severity, s));
        }
        // A lone "profile" banner with nothing else at the requested
        // severity isn't informative — it just says "shape=X, no
        // warns/misses". Suppress to keep the output clean; users
        // see the banner when they run without --min-severity.
        let has_actionable = findings.iter().any(|f| f.code != "profile");
        if has_actionable {
            reports.push(Report {
                entity: name,
                detected_profile,
                effective_profile,
                findings,
            });
        }
    }

    if as_json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        if fail_on_tripped {
            // Even in JSON mode, --fail-on still drives the exit code.
            // We've already emitted the report so consumers can read
            // the findings; the bail just sets exit != 0.
            anyhow::bail!("存在达到 --fail-on 指定级别的 lint 结果");
        }
        return Ok(());
    }

    if reports.is_empty() {
        println!("没有达到指定严重级别的检查结果。");
        if fail_on_tripped {
            anyhow::bail!(
                "存在达到 --fail-on 指定级别的 lint 结果（已被 --min-severity 筛选隐藏）",
            );
        }
        return Ok(());
    }

    for r in &reports {
        println!("\n── {} ──", r.entity);
        let mut table = Table::new();
        table.load_style(UTF8_FULL).set_header(vec![
            Cell::new("").fg(Color::Cyan),
            Cell::new("类别").fg(Color::Cyan),
            Cell::new("检查项").fg(Color::Cyan),
            Cell::new("说明").fg(Color::Cyan),
        ]);
        for f in &r.findings {
            let (symbol, color) = match f.severity {
                LintSeverity::Pass => ("✓", Color::Green),
                LintSeverity::Warn => ("!", Color::Yellow),
                LintSeverity::Miss => ("✗", Color::Red),
            };
            // Inline the ABAP CDS hint under the message so the user
            // sees both without a separate column. Keeps the table
            // from blowing out horizontally on actionable findings.
            let mut msg = f.message.clone();
            if let Some(cds) = f.suggested_cds {
                msg.push_str("\nABAP CDS 建议：");
                msg.push_str(cds);
            }
            if let Some(why) = f.why_in_fiori {
                msg.push_str("\n→ ");
                msg.push_str(why);
            }
            table.add_row(vec![
                Cell::new(symbol).fg(color),
                Cell::new(match f.category {
                    lint::LintCategory::Profile => "配置类型",
                    lint::LintCategory::Identity => "身份标识",
                    lint::LintCategory::ListReport => "列表报表",
                    lint::LintCategory::Filtering => "筛选",
                    lint::LintCategory::Fields => "字段",
                    lint::LintCategory::Capabilities => "功能能力",
                    lint::LintCategory::Integrity => "完整性",
                }),
                Cell::new(f.code),
                Cell::new(msg),
            ]);
        }
        println!("{table}");
    }
    if fail_on_tripped {
        anyhow::bail!("存在达到 --fail-on 指定级别的 lint 结果");
    }
    Ok(())
}

/// Capture the bytes of a connected service's `$metadata` into the
/// offline library. Mirrors the Tauri command but is a separate
/// implementation because the CLI flow operates without `AppState`:
/// load config from disk → call the core save function → write back.
/// The core function does the atomic EDMX + TOML write internally.
async fn cmd_offline_save(
    client: &SapClient,
    connected_profile_name: &str,
    service_path: &str,
    offline_profile_name: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
    json: bool,
) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;
    let base_url = cfg
        .connections
        .get(connected_profile_name)
        .map(|p| p.base_url.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "connections.toml 中找不到连接配置 '{}'",
                connected_profile_name
            )
        })?;
    let svc_trimmed = service_path.trim_start_matches('/').trim_end_matches('/');
    let source_url = format!(
        "{}/{}/$metadata",
        base_url.trim_end_matches('/'),
        svc_trimmed
    );

    let opts = SaveOptions {
        offline_profile_name: offline_profile_name
            .or_else(|| Some(auto_offline_profile_name(connected_profile_name))),
        source_profile_for_new_bucket: Some(connected_profile_name.to_string()),
        label_override,
        note,
        now_iso: current_iso8601(),
    };

    let outcome = offline::save_service_offline(
        client,
        service_path,
        source_url,
        &mut cfg,
        &config_dir.path,
        opts,
    )
    .await
    .context("保存服务到离线库失败")?;

    if json {
        let s = serde_json::to_string_pretty(&outcome)?;
        println!("{s}");
        return Ok(());
    }

    render_offline_save_outcome(&outcome);
    Ok(())
}

fn render_offline_save_outcome(outcome: &SaveOutcome) {
    use offline::SaveKind;
    let header = match outcome.kind {
        SaveKind::NewService => "已保存到离线库（新条目）",
        SaveKind::OverwriteUpdatedBytes => "已保存到离线库（更新现有条目）",
        SaveKind::SkippedByteIdentical => "已跳过——文件内容与现有条目相同",
    };
    println!("{header}");
    println!("  配置：          {}", outcome.offline_profile_name);
    println!("  服务 ID：       {}", outcome.service_id);
    println!("  EDMX 文件：     {}", outcome.edmx_file);
    println!("  OData 版本：    {:?}", outcome.odata_version);
    println!("  SHA-256：       {}", outcome.sha256);
    println!("  大小：          {} 字节", outcome.size_bytes);
    if outcome.created_new_offline_profile {
        println!("  （已创建新的离线配置桶）");
    }
}

/// Path B: import an EDMX file from disk. No SapClient — the file
/// path is read directly and run through the import-validation
/// pipeline. Reuses the same atomic-write / lockfile primitives as
/// `cmd_offline_save`.
fn cmd_offline_import(
    file: std::path::PathBuf,
    target_offline_profile: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
    json: bool,
) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;
    let opts = ImportOptions {
        file_path: file,
        target_offline_profile,
        label_override,
        note,
        now_iso: current_iso8601(),
    };
    let outcome = offline::import_edmx_file(&mut cfg, &config_dir.path, opts)
        .context("导入 EDMX 文件失败")?;
    if json {
        let s = serde_json::to_string_pretty(&outcome)?;
        println!("{s}");
        return Ok(());
    }
    render_offline_save_outcome(&outcome);
    Ok(())
}

/// List offline profiles (when `profile` is None) or services within
/// an offline profile. Reads only the local TOML index — no network.
fn cmd_offline_list(profile: Option<String>, json: bool) -> Result<()> {
    let (cfg, _) = config::load_config().context("加载配置文件失败")?;
    match profile {
        None => render_offline_profiles(&cfg, json),
        Some(name) => render_offline_services(&cfg, &name, json),
    }
}

fn render_offline_profiles(cfg: &config::ConfigFile, json: bool) -> Result<()> {
    if json {
        let entries: Vec<_> = cfg
            .offline_profiles
            .iter()
            .map(|(name, p)| {
                serde_json::json!({
                    "name": name,
                    "source_profile": p.source_profile,
                    "created_at": p.created_at,
                    "service_count": cfg
                        .offline_services
                        .iter()
                        .filter(|s| &s.profile == name)
                        .count(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    if cfg.offline_profiles.is_empty() {
        println!(
            "尚无离线配置。可使用 `sap-odata offline save`（需连接配置）或 `sap-odata offline import <file>` 创建。"
        );
        return Ok(());
    }
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL)
        .set_header(vec!["配置", "来源", "服务数", "创建时间"]);
    for (name, p) in &cfg.offline_profiles {
        let count = cfg
            .offline_services
            .iter()
            .filter(|s| &s.profile == name)
            .count();
        let source = if p.source_profile.is_empty() {
            "—"
        } else {
            &p.source_profile
        };
        table.add_row(vec![
            Cell::new(name),
            Cell::new(source),
            Cell::new(count.to_string()),
            Cell::new(&p.created_at),
        ]);
    }
    println!("{table}");
    Ok(())
}

/// Error for an `offline list/delete --profile NAME` that names no
/// offline bucket. The common trap is a habitual global `-p DEV`: clap
/// shares the `profile` id between the global flag and the offline
/// subcommands' `--profile`, so the connected profile name arrives as
/// the bucket name. Say so instead of a bare "not found".
fn offline_bucket_not_found(cfg: &config::ConfigFile, name: &str) -> anyhow::Error {
    let buckets: Vec<&str> = cfg.offline_profiles.keys().map(String::as_str).collect();
    let available = if buckets.is_empty() {
        "暂无".to_string()
    } else {
        buckets.join(", ")
    };
    if cfg.connections.contains_key(name) {
        anyhow::anyhow!(
            "'{name}' 是连接配置，不是离线配置桶。离线命令通过 --profile 指定配置桶，而全局 -p 也会填入同一个参数——请省略 -p 或传入配置桶名称。可用离线配置桶：{available}"
        )
    } else {
        anyhow::anyhow!("找不到离线配置 '{name}'。可用离线配置桶：{available}")
    }
}

fn render_offline_services(cfg: &config::ConfigFile, profile: &str, json: bool) -> Result<()> {
    if !cfg.offline_profiles.contains_key(profile) {
        return Err(offline_bucket_not_found(cfg, profile));
    }
    let services: Vec<_> = cfg
        .offline_services
        .iter()
        .filter(|s| s.profile == profile)
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&services)?);
        return Ok(());
    }
    if services.is_empty() {
        println!("离线配置 '{profile}' 中尚无服务。");
        return Ok(());
    }
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL)
        .set_header(vec!["服务 ID", "标签", "版本", "大小", "来源"]);
    for s in services {
        let source = s
            .source_service_path
            .clone()
            .or_else(|| s.original_filename.clone())
            .unwrap_or_else(|| "—".to_string());
        table.add_row(vec![
            Cell::new(&s.id),
            Cell::new(&s.label),
            Cell::new(&s.odata_version),
            Cell::new(format!("{} 字节", s.size_bytes)),
            Cell::new(truncate_for_table(&source, 60)),
        ]);
    }
    println!("{table}");
    Ok(())
}

/// Delete an offline service or a whole offline-profile bucket.
/// Reads only the local TOML index — no network. Confirms
/// interactively unless `--yes` is passed, mirroring the GUI's
/// confirmation dialog. Goes through the same core delete helpers
/// (`offline::delete_offline_service` / `delete_offline_profile`)
/// as the Tauri command, so the boundary discipline is identical.
fn cmd_offline_delete(
    profile: String,
    service_id: Option<String>,
    yes: bool,
    json: bool,
) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("加载配置文件失败")?;

    // Pre-flight existence check so the confirmation prompt only
    // appears when there's actually something to delete. Without
    // this, the user gets the prompt, types `y`, and only then
    // discovers the profile didn't exist.
    if !cfg.offline_profiles.contains_key(&profile) {
        return Err(offline_bucket_not_found(&cfg, &profile));
    }
    if let Some(ref id) = service_id {
        let exists = cfg
            .offline_services
            .iter()
            .any(|s| s.profile == profile && &s.id == id);
        if !exists {
            anyhow::bail!(
                "离线配置 '{profile}' 中找不到服务 '{id}'。运行 `sap-odata offline list --profile {profile}` 查看可用服务 ID。"
            );
        }
    }

    if !yes {
        let prompt = match &service_id {
            Some(id) => format!(
                "确定从 '{profile}' 中删除离线服务 '{id}' 吗？缓存的 EDMX 文件也会被删除。（y/N）"
            ),
            None => {
                let count = cfg
                    .offline_services
                    .iter()
                    .filter(|s| s.profile == profile)
                    .count();
                format!(
                    "确定删除离线配置 '{profile}'（包含 {count} 个服务）吗？此配置桶中的所有 EDMX 缓存文件都会被删除。（y/N）"
                )
            }
        };
        use dialoguer::Confirm;
        let confirmed = Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()?;
        if !confirmed {
            anyhow::bail!("已取消操作。");
        }
    }

    match service_id {
        Some(id) => {
            let outcome =
                offline::delete_offline_service(&mut cfg, &config_dir.path, &profile, &id)
                    .context("删除离线服务失败")?;
            if json {
                let payload = serde_json::json!({
                    "profile": outcome.profile,
                    "service_id": outcome.service_id,
                    "file_removed": outcome.file_removed,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!(
                    "已从离线配置 '{}' 中删除服务 '{}'（文件状态：{}）",
                    outcome.profile,
                    outcome.service_id,
                    if outcome.file_removed {
                        "已删除"
                    } else {
                        "原已不存在"
                    }
                );
            }
        }
        None => {
            let outcome = offline::delete_offline_profile(&mut cfg, &config_dir.path, &profile)
                .context("删除离线配置失败")?;
            if json {
                let payload = serde_json::json!({
                    "profile": outcome.profile,
                    "services_removed": outcome.services_removed,
                    "files_removed": outcome.files_removed,
                    "directory_removed": outcome.directory_removed,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!(
                    "已删除离线配置 '{}'（{} 个服务、{} 个文件{}）",
                    outcome.profile,
                    outcome.services_removed,
                    outcome.files_removed,
                    if outcome.directory_removed {
                        "，配置桶目录已删除"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    Ok(())
}

/// Severity ranking helper: Pass < Warn < Miss. Returns true if
/// `actual` is at or above `threshold`.
fn severity_at_least(actual: LintSeverity, threshold: LintSeverity) -> bool {
    fn rank(s: LintSeverity) -> u8 {
        match s {
            LintSeverity::Pass => 0,
            LintSeverity::Warn => 1,
            LintSeverity::Miss => 2,
        }
    }
    rank(actual) >= rank(threshold)
}

// ── Helpers ──

/// Whether a metadata identifier (entity-set name, property name, response
/// column) belongs to the SAP V4 framework's reserved-prefix family. The
/// `SAP__` and `__` double-underscore prefixes are owned by the V4
/// runtime — `SAP__Messages` (transient message container),
/// `SAP__Currencies`, `SAP__UnitsOfMeasure`, `__OperationControl` — none
/// of which are part of the developer's data model. Table output filters
/// these out for a cleaner read; `--json` output is left untouched so
/// scripted consumers still see the raw service truth. The same rule
/// lives in the desktop frontend (services.js / describe.js / results.js).
fn is_sap_system_field(name: &str) -> bool {
    name.starts_with("SAP__") || name.starts_with("__")
}

fn extract_results(data: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(d) = data.get("d") {
        if let Some(results) = d.get("results") {
            return Some(results.clone());
        }
        return Some(d.clone());
    }
    if let Some(value) = data.get("value") {
        return Some(value.clone());
    }
    None
}

fn print_results_table(rows: &[serde_json::Value]) {
    let Some(first) = rows.first() else { return };
    let Some(obj) = first.as_object() else {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_default()
        );
        return;
    };

    let columns: Vec<&String> = obj
        .keys()
        .filter(|k| {
            !k.starts_with('@')
                && !is_sap_system_field(k)
                && !matches!(
                    obj.get(*k),
                    Some(serde_json::Value::Object(_) | serde_json::Value::Array(_))
                )
        })
        .collect();

    let mut table = Table::new();
    table.load_style(UTF8_FULL);
    table.set_header(
        columns
            .iter()
            .map(|c| Cell::new(c.as_str()).fg(Color::DarkCyan)),
    );

    for row in rows {
        let cells: Vec<Cell> = columns
            .iter()
            .map(|col| {
                let val = row.get(*col);
                let text = match val {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Null) => String::new(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                Cell::new(text)
            })
            .collect();
        table.add_row(cells);
    }

    println!("{table}");
    println!("\n  返回 {} 行\n", rows.len());
}
