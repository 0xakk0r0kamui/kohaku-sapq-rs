//! Tornadocash relayer process management.

use std::{ops::Deref, process::Stdio, time::Duration};

use alloy::{
    primitives::{Address, U256},
    providers::{DynProvider, Provider},
    signers::local::PrivateKeySigner,
};
use anyhow::bail;
use kohaku_tornadocash::Pool;
use kohaku_tornadocash_relayer::Relayer;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    time::timeout,
};

const DEFAULT_PORT: u16 = 8000;
const MAX_START_ATTEMPTS: u16 = 20;
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const PREFUND_ETH: u128 = 1_000 * 10_u128.pow(18);

/// Builder for a locally hosted [`Relayer`] stack.
pub struct RelayerBuilder {
    http_rpc_url: String,
    ws_rpc_url: String,
    pool: Pool,
    proxy_address: Address,
    private_key: String,
    reward_account: Address,
}

/// A running relayer stack and a connected [`Relayer`].
pub struct RelayerInstance {
    redis: Child,
    server: Child,
    worker: Child,
    relayer: Relayer,
}

impl RelayerBuilder {
    pub fn new(
        http_rpc_url: impl Into<String>,
        ws_rpc_url: impl Into<String>,
        pool: Pool,
        proxy_address: Address,
        private_key: impl Into<String>,
        reward_account: Address,
    ) -> Self {
        Self {
            http_rpc_url: http_rpc_url.into(),
            ws_rpc_url: ws_rpc_url.into(),
            pool,
            proxy_address,
            private_key: private_key.into(),
            reward_account,
        }
    }

    /// Funds the relayer's signer with native ETH.
    ///
    /// # Errors
    /// Returns an error if the private key fails to parse, or the RPC request fails.
    pub async fn prefund(self, provider: &DynProvider) -> Result<Self, anyhow::Error> {
        let signer: PrivateKeySigner = self.private_key.parse()?;
        provider
            .raw_request::<_, ()>(
                "anvil_setBalance".into(),
                (signer.address(), U256::from(PREFUND_ETH)),
            )
            .await?;
        Ok(self)
    }

    /// Spawns the relayer stack.
    ///
    /// # Errors
    /// Returns an error if the relayer fails to start on every attempted port.
    pub async fn spawn(self) -> Result<RelayerInstance, anyhow::Error> {
        let mut port = DEFAULT_PORT;

        for attempt in 1..=MAX_START_ATTEMPTS {
            match self.try_spawn(port).await {
                Ok(instance) => return Ok(instance),
                Err(err) if attempt < MAX_START_ATTEMPTS => {
                    tracing::warn!(
                        "relayer failed to start on port {port} (attempt {attempt}/{MAX_START_ATTEMPTS}): {err}"
                    );
                    port += 1;
                }
                Err(err) => return Err(err),
            }
        }

        bail!("relayer failed to start relayer after {MAX_START_ATTEMPTS} attempts");
    }

    async fn try_spawn(&self, port: u16) -> Result<RelayerInstance, anyhow::Error> {
        let redis_port = port + 1;
        let mut redis = Command::new("redis-server")
            .arg("--port")
            .arg(redis_port.to_string())
            .arg("--save")
            .arg("")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        if let Err(err) = wait_for_redis(redis_port).await {
            let _ = redis.start_kill();
            return Err(err);
        }

        let redis_url = format!("redis://127.0.0.1:{redis_port}");
        let envs = self.envs(port, &redis_url);

        let mut server = Command::new("tornado-relayer-server")
            .envs(envs.clone())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        tokio::spawn(forward_lines(
            server.stdout.take().unwrap(),
            "relayer-server",
        ));
        tokio::spawn(forward_lines(
            server.stderr.take().unwrap(),
            "relayer-server",
        ));

        let mut worker = Command::new("tornado-relayer-worker")
            .envs(envs)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        tokio::spawn(forward_lines(
            worker.stdout.take().unwrap(),
            "relayer-worker",
        ));
        tokio::spawn(forward_lines(
            worker.stderr.take().unwrap(),
            "relayer-worker",
        ));

        let relayer = Relayer::new(&format!("http://127.0.0.1:{port}"));

        if timeout(READY_TIMEOUT, wait_ready(&relayer)).await.is_err() {
            let _ = redis.start_kill();
            let _ = server.start_kill();
            let _ = worker.start_kill();
            anyhow::bail!("timed out waiting for relayer server on port {port}");
        }

        Ok(RelayerInstance {
            redis,
            server,
            worker,
            relayer,
        })
    }

    fn envs(&self, port: u16, redis_url: &str) -> Vec<(String, String)> {
        vec![
            ("NET_ID".into(), self.pool.chain_id.to_string()),
            ("HTTP_RPC_URL".into(), self.http_rpc_url.clone()),
            ("WS_RPC_URL".into(), self.ws_rpc_url.clone()),
            ("ORACLE_RPC_URL".into(), self.http_rpc_url.clone()),
            ("REDIS_URL".into(), redis_url.to_string()),
            (
                "PRIVATE_KEY".into(),
                self.private_key
                    .strip_prefix("0x")
                    .unwrap_or(&self.private_key)
                    .to_string(),
            ),
            ("REWARD_ACCOUNT".into(), self.reward_account.to_string()),
            ("REGULAR_TORNADO_WITHDRAW_FEE".into(), "0.05".into()),
            ("MINING_SERVICE_FEE".into(), "0.05".into()),
            ("CONFIRMATIONS".into(), "1".into()),
            ("MAX_GAS_PRICE".into(), "1000".into()),
            ("BASE_FEE_RESERVE_PERCENTAGE".into(), "25".into()),
            ("APP_PORT".into(), port.to_string()),
            ("LOCAL_POOL_ADDRESS".into(), self.pool.address.to_string()),
            ("LOCAL_POOL_SYMBOL".into(), self.pool.symbol().to_string()),
            ("LOCAL_POOL_AMOUNT".into(), self.pool.amount()),
            (
                "LOCAL_POOL_DECIMALS".into(),
                self.pool.asset.decimals().to_string(),
            ),
            ("LOCAL_PROXY_ADDRESS".into(), self.proxy_address.to_string()),
        ]
    }
}

/// Polls `redis_port` until a TCP connection succeeds.
async fn wait_for_redis(redis_port: u16) -> Result<(), anyhow::Error> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", redis_port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for redis-server on port {redis_port}");
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }
}

/// Forwards every line from `stream` to `tracing` for as long as the process runs.
async fn forward_lines(stream: impl AsyncRead + Unpin, label: &'static str) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::info!("{label}: {line}");
    }
}

/// Polls the relayer's `/v1/status` endpoint until it responds.
async fn wait_ready(relayer: &Relayer) {
    loop {
        if relayer.status().await.is_ok() {
            return;
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }
}

impl Deref for RelayerInstance {
    type Target = Relayer;

    fn deref(&self) -> &Self::Target {
        &self.relayer
    }
}

impl Drop for RelayerInstance {
    fn drop(&mut self) {
        let _ = self.redis.start_kill();
        let _ = self.server.start_kill();
        let _ = self.worker.start_kill();
    }
}
