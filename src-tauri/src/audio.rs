use std::io::Cursor;

use anyhow::{Context, Result, anyhow, bail};
use mp3lame_encoder::{
    Bitrate, Builder, Encoder, FlushGap, InterleavedPcm, Quality, max_required_buffer_size,
};
use opus_pure::OpusDecoder;
use symphonia::core::{
    audio::{SampleBuffer, SignalSpec},
    codecs::{CODEC_TYPE_AAC, CODEC_TYPE_OPUS, CodecParameters, DecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::default::{get_codecs, get_probe};

const OPUS_SAMPLE_RATE: u32 = 48_000;
const OPUS_MAX_FRAME_SIZE: usize = 5_760;

pub fn m4a_to_mp3(input: &[u8]) -> Result<Vec<u8>> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(input.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let format_options = FormatOptions {
        enable_gapless: true,
        ..FormatOptions::default()
    };
    let probed = get_probe()
        .format(&hint, source, &format_options, &MetadataOptions::default())
        .context("无法识别 M4A 音频")?;
    let mut format = probed.format;
    let track = format.default_track().context("M4A 中没有音频轨道")?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    match codec_params.codec {
        CODEC_TYPE_OPUS => opus_to_mp3(&mut *format, track_id, &codec_params),
        CODEC_TYPE_AAC => aac_to_mp3(&mut *format, track_id, &codec_params),
        codec => bail!("暂不支持此音频编码（{codec:?}）"),
    }
}

fn opus_to_mp3(
    format: &mut dyn FormatReader,
    track_id: u32,
    codec_params: &CodecParameters,
) -> Result<Vec<u8>> {
    let channels = opus_channels(codec_params)?;
    validate_channels(channels)?;

    let mut decoder = OpusDecoder::new(OPUS_SAMPLE_RATE as i32, channels)
        .map_err(|error| anyhow!("无法创建 Opus 音频解码器：{error}"))?;
    let mut encoder = build_encoder(channels, OPUS_SAMPLE_RATE)?;
    let mut pcm = vec![0i16; OPUS_MAX_FRAME_SIZE * channels];
    let mut output = Vec::new();
    let mut decoded_any = false;

    loop {
        let packet = match next_audio_packet(format, track_id)? {
            Some(packet) => packet,
            None => break,
        };
        let frames = decoder
            .decode_s16(packet.buf(), OPUS_MAX_FRAME_SIZE, &mut pcm)
            .map_err(|error| anyhow!("Opus 音频解码失败：{error}"))?;
        let trim_start = (packet.trim_start() as usize).min(frames);
        let trim_end = (packet.trim_end() as usize).min(frames.saturating_sub(trim_start));
        let first = trim_start * channels;
        let last = (frames - trim_end) * channels;
        if first >= last {
            continue;
        }
        encode_pcm(&mut encoder, channels, &pcm[first..last], &mut output)?;
        decoded_any = true;
    }

    finish_mp3(encoder, decoded_any, output)
}

fn opus_channels(codec_params: &CodecParameters) -> Result<usize> {
    if let Some(channels) = codec_params.channels {
        return Ok(channels.count());
    }
    let header = codec_params
        .extra_data
        .as_deref()
        .context("Opus 音频缺少声道信息")?;
    if !header.starts_with(b"OpusHead") || header.len() < 10 {
        bail!("Opus 音频声道信息无效");
    }
    Ok(header[9] as usize)
}

fn aac_to_mp3(
    format: &mut dyn FormatReader,
    track_id: u32,
    codec_params: &CodecParameters,
) -> Result<Vec<u8>> {
    let mut decoder = get_codecs()
        .make(codec_params, &DecoderOptions::default())
        .context("无法创建 AAC 音频解码器")?;
    let mut encoder = None;
    let mut expected_spec: Option<SignalSpec> = None;
    let mut output = Vec::new();
    let mut decoded_any = false;

    loop {
        let packet = match next_audio_packet(format, track_id)? {
            Some(packet) => packet,
            None => break,
        };
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error).context("AAC 音频解码失败"),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        validate_channels(channels)?;
        if let Some(previous) = expected_spec {
            if previous.rate != spec.rate || previous.channels != spec.channels {
                bail!("音频参数在文件中途发生变化");
            }
        } else {
            expected_spec = Some(spec);
            encoder = Some(build_encoder(channels, spec.rate)?);
        }

        let mut samples = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        encode_pcm(
            encoder.as_mut().context("MP3 编码器尚未初始化")?,
            channels,
            samples.samples(),
            &mut output,
        )?;
        decoded_any = true;
    }

    finish_mp3(encoder.context("M4A 音频内容为空")?, decoded_any, output)
}

fn next_audio_packet(
    format: &mut dyn FormatReader,
    track_id: u32,
) -> Result<Option<symphonia::core::formats::Packet>> {
    loop {
        match format.next_packet() {
            Ok(packet) if packet.track_id() == track_id => return Ok(Some(packet)),
            Ok(_) => continue,
            Err(SymphoniaError::ResetRequired) => bail!("音频参数在文件中途发生变化"),
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error).context("读取 M4A 音频失败"),
        }
    }
}

fn validate_channels(channels: usize) -> Result<()> {
    if channels != 1 && channels != 2 {
        bail!("暂不支持 {channels} 声道音频");
    }
    Ok(())
}

fn build_encoder(channels: usize, sample_rate: u32) -> Result<Encoder> {
    Builder::new()
        .context("无法初始化 MP3 编码器")?
        .with_num_channels(channels as u8)
        .map_err(|error| anyhow!("无法设置 MP3 声道：{error:?}"))?
        .with_sample_rate(sample_rate)
        .map_err(|error| anyhow!("无法设置 MP3 采样率：{error:?}"))?
        .with_brate(Bitrate::Kbps192)
        .map_err(|error| anyhow!("无法设置 MP3 码率：{error:?}"))?
        .with_quality(Quality::Best)
        .map_err(|error| anyhow!("无法设置 MP3 质量：{error:?}"))?
        .with_to_write_vbr_tag(false)
        .map_err(|error| anyhow!("无法关闭 MP3 可变码率标签：{error:?}"))?
        .build()
        .map_err(|error| anyhow!("无法启动 MP3 编码器：{error:?}"))
}

fn encode_pcm(
    encoder: &mut Encoder,
    channels: usize,
    pcm: &[i16],
    output: &mut Vec<u8>,
) -> Result<()> {
    let frames = pcm.len() / channels;
    output.reserve(max_required_buffer_size(frames));
    if channels == 1 {
        encoder
            .encode_to_vec(mp3lame_encoder::MonoPcm(pcm), output)
            .map_err(|error| anyhow!("MP3 编码失败：{error:?}"))?;
    } else {
        encoder
            .encode_to_vec(InterleavedPcm(pcm), output)
            .map_err(|error| anyhow!("MP3 编码失败：{error:?}"))?;
    }
    Ok(())
}

fn finish_mp3(mut encoder: Encoder, decoded_any: bool, mut output: Vec<u8>) -> Result<Vec<u8>> {
    if !decoded_any {
        bail!("M4A 音频内容为空");
    }
    output.reserve(7_200);
    encoder
        .flush_to_vec::<FlushGap>(&mut output)
        .map_err(|error| anyhow!("MP3 编码收尾失败：{error:?}"))?;
    if output.len() < 4 || !output.starts_with(&[0xff, 0xfb]) {
        bail!("MP3 转换结果无效");
    }
    Ok(output)
}
