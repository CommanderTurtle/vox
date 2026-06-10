//! 阿里云智能语音交互 — 一句话识别 REST API。
//!
//! API: POST https://nls-gateway.{region}.aliyuncs.com/stream/v1/asr
//!
//! 鉴权：使用 appkey + token（通过阿里云 AK 获取的临时 token）
//! 音频格式：16kHz 单声道 PCM（需要从 WAV 中提取原始 PCM 数据）

use async_trait::async_trait;

use crate::asr::{AsrEngine, AsrError};

/// 阿里云一句话识别 ASR 引擎。
pub struct AliyunAsrEngine {
    appkey: String,
    token: String,
    /// 服务区域 (默认 cn-shanghai)
    region: String,
    client: reqwest::Client,
}

impl AliyunAsrEngine {
    pub fn new(appkey: &str, token: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            appkey: appkey.to_string(),
            token: token.to_string(),
            region: "cn-shanghai".to_string(),
            client,
        }
    }
}

#[async_trait]
impl AsrEngine for AliyunAsrEngine {
    fn name(&self) -> &'static str {
        "aliyun"
    }

    async fn transcribe(&self, audio_wav: &[u8]) -> Result<String, AsrError> {
        // 从 WAV 中提取原始 PCM 数据（跳过 WAV 头部 44 字节）
        let pcm_data = if audio_wav.len() > 44 {
            audio_wav[44..].to_vec()
        } else {
            return Err(AsrError::AudioFormat("WAV file too short".to_string()));
        };

        let url = format!(
            "https://nls-gateway.{}.aliyuncs.com/stream/v1/asr?appkey={}&Format=pcm&SampleRate=16000&EnableIntermediateResult=false",
            self.region, self.appkey
        );

        let response = self
            .client
            .post(&url)
            .header("X-NLS-Token", &self.token)
            .header("Content-Type", "application/octet-stream")
            .body(pcm_data)
            .send()
            .await
            .map_err(|e| AsrError::EngineError {
                engine: "aliyun".into(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| AsrError::EngineError {
            engine: "aliyun".into(),
            message: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            return Err(AsrError::EngineError {
                engine: "aliyun".into(),
                message: format!("API returned {}: {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()),
            });
        }

        // 解析响应
        #[derive(serde::Deserialize)]
        struct AliyunAsrResponse {
            status: i32,
            result: Option<String>,
            message: Option<String>,
        }

        let resp: AliyunAsrResponse = serde_json::from_str(&body).map_err(|e| {
            AsrError::EngineError {
                engine: "aliyun".into(),
                message: format!(
                    "Failed to parse response JSON: {} — body: {}",
                    e,
                    body.chars().take(200).collect::<String>()
                ),
            }
        })?;

        if resp.status != 20000000 {
            return Err(AsrError::EngineError {
                engine: "aliyun".into(),
                message: format!(
                    "API error {}: {}",
                    resp.status,
                    resp.message.unwrap_or_default()
                ),
            });
        }

        Ok(resp.result.unwrap_or_default().trim().to_string())
    }
}
