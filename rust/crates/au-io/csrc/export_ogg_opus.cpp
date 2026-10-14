// SPDX-License-Identifier: GPL-3.0-or-later
// Audacity 3.7.3 Ogg Vorbis and Opus export shim.

#include <ogg/ogg.h>
#include <opus/opus.h>
#include <opus/opus_multistream.h>
#include <vorbis/vorbisenc.h>

#include <algorithm>
#include <cassert>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <exception>
#include <limits>
#include <random>
#include <stdexcept>
#include <string>
#include <vector>

extern "C" {

struct AudacityTag {
    const char *name;
    const char *value;
};

struct AudacityVorbisOptions {
    int32_t quality;
};

struct AudacityOpusOptions {
    int32_t bitrate;
    int32_t quality;
    int32_t frame_multiplier;
    int32_t vbr_mode;
    int32_t application;
    int32_t cutoff;
};

int audacity_export_vorbis(
    const char *path,
    uint32_t sample_rate,
    uint32_t channels,
    const float *interleaved,
    uint64_t frames,
    const AudacityVorbisOptions *options,
    const AudacityTag *tags,
    size_t tag_count,
    char *error_message,
    size_t error_message_size);

int audacity_export_opus(
    const char *path,
    uint32_t sample_rate,
    uint32_t channels,
    const float *interleaved,
    uint64_t frames,
    const AudacityOpusOptions *options,
    const AudacityTag *tags,
    size_t tag_count,
    char *error_message,
    size_t error_message_size);
}

namespace {

void SetError(char *buffer, size_t size, const char *message) noexcept
{
    if (buffer == nullptr || size == 0)
        return;
    std::snprintf(buffer, size, "%s", message == nullptr ? "Export failed" : message);
}

void Check(bool condition, const char *message)
{
    if (!condition)
        throw std::runtime_error(message);
}

void CheckOggWrite(FILE *file, const unsigned char *data, size_t size)
{
    Check(std::fwrite(data, 1, size, file) == size, "Unable to write Ogg data");
}

bool IsYearTag(const char *name)
{
    if (name == nullptr)
        return false;
    return std::strcmp(name, "YEAR") == 0 ||
           std::strcmp(name, "Year") == 0 ||
           std::strcmp(name, "year") == 0;
}

struct VorbisState {
    vorbis_info info{};
    vorbis_comment comment{};
    vorbis_dsp_state dsp{};
    vorbis_block block{};
    ogg_stream_state stream{};
    FILE *file{nullptr};
    bool info_initialized{false};
    bool comment_initialized{false};
    bool dsp_initialized{false};
    bool block_initialized{false};
    bool stream_initialized{false};

    VorbisState()
    {
        vorbis_info_init(&info);
        info_initialized = true;
    }

    ~VorbisState()
    {
        if (stream_initialized)
            ogg_stream_clear(&stream);
        if (comment_initialized)
            vorbis_comment_clear(&comment);
        if (block_initialized)
            vorbis_block_clear(&block);
        if (dsp_initialized)
            vorbis_dsp_clear(&dsp);
        if (info_initialized)
            vorbis_info_clear(&info);
        if (file != nullptr)
            std::fclose(file);
    }
};

void WriteVorbisPage(FILE *file, const ogg_page &page)
{
    CheckOggWrite(file, page.header, static_cast<size_t>(page.header_len));
    CheckOggWrite(file, page.body, static_cast<size_t>(page.body_len));
}

void WriteVorbisHeaders(VorbisState &state)
{
    ogg_packet header{};
    ogg_packet comment{};
    ogg_packet codebook{};

    Check(vorbis_analysis_headerout(
              &state.dsp, &state.comment, &header, &comment, &codebook) == 0,
          "Unable to create Vorbis headers");
    Check(ogg_stream_packetin(&state.stream, &header) == 0,
          "Unable to add Vorbis header packet");
    Check(ogg_stream_packetin(&state.stream, &comment) == 0,
          "Unable to add Vorbis comment packet");
    Check(ogg_stream_packetin(&state.stream, &codebook) == 0,
          "Unable to add Vorbis codebook packet");

    // Flush the three headers so audio begins on a new Ogg page.
    ogg_page page{};
    while (ogg_stream_flush(&state.stream, &page))
        WriteVorbisPage(state.file, page);
}

void AddVorbisComments(
    vorbis_comment *comment,
    const AudacityTag *tags,
    size_t tag_count)
{
    vorbis_comment_init(comment);
    for (size_t i = 0; i < tag_count; ++i) {
        const char *name = tags[i].name == nullptr ? "" : tags[i].name;
        const char *value = tags[i].value == nullptr ? "" : tags[i].value;
        vorbis_comment_add_tag(
            comment,
            const_cast<char *>(IsYearTag(name) ? "DATE" : name),
            const_cast<char *>(value));
    }
}

int ExportVorbis(
    const char *path,
    uint32_t sample_rate,
    uint32_t channels,
    const float *interleaved,
    uint64_t frames,
    const AudacityVorbisOptions *options,
    const AudacityTag *tags,
    size_t tag_count)
{
    constexpr size_t SAMPLES_PER_RUN = 8192u;

    VorbisState state;
    const double quality =
        static_cast<double>(std::clamp(options->quality, 0, 10)) / 10.0;

    if (vorbis_encode_init_vbr(
            &state.info,
            static_cast<long>(channels),
            static_cast<long>(sample_rate + 0.5),
            quality)) {
        throw std::runtime_error("Unable to export - rate or quality problem");
    }

    state.file = std::fopen(path, "wb");
    Check(state.file != nullptr, "Unable to open target file for writing");

    Check(vorbis_analysis_init(&state.dsp, &state.info) == 0,
          "Unable to initialize Vorbis analysis");
    state.dsp_initialized = true;
    Check(vorbis_block_init(&state.dsp, &state.block) == 0,
          "Unable to initialize Vorbis block");
    state.block_initialized = true;

    AddVorbisComments(&state.comment, tags, tag_count);
    state.comment_initialized = true;

    // Match Audacity's serial-number initialization and rand() call point.
    std::srand(static_cast<unsigned>(std::time(nullptr)));
    Check(ogg_stream_init(&state.stream, std::rand()) == 0,
          "Unable to create Ogg stream");
    state.stream_initialized = true;

    WriteVorbisHeaders(state);

    uint64_t position = 0;
    bool eos = false;
    while (!eos) {
        float **vorbis_buffer =
            vorbis_analysis_buffer(&state.dsp, static_cast<int>(SAMPLES_PER_RUN));
        Check(vorbis_buffer != nullptr, "Unable to allocate Vorbis analysis buffer");

        const size_t samples_this_run =
            static_cast<size_t>(std::min<uint64_t>(
                SAMPLES_PER_RUN, frames - position));

        for (uint32_t channel = 0; channel < channels; ++channel) {
            for (size_t i = 0; i < SAMPLES_PER_RUN; ++i) {
                const float sample =
                    i < samples_this_run
                        ? interleaved[(position + i) * channels + channel]
                        : 0.0f;
                vorbis_buffer[channel][i] = sample;
            }
        }

        const int analysis_result = vorbis_analysis_wrote(
            &state.dsp, static_cast<int>(samples_this_run));
        Check(analysis_result == 0, "Vorbis analysis failed");

        ogg_packet packet{};
        ogg_page page{};
        while (vorbis_analysis_blockout(&state.dsp, &state.block) == 1) {
            int error = vorbis_analysis(&state.block, nullptr);
            if (!error)
                error = vorbis_bitrate_addblock(&state.block);

            while (!error && vorbis_bitrate_flushpacket(&state.dsp, &packet)) {
                error = ogg_stream_packetin(&state.stream, &packet);
                while (!error && !eos) {
                    const int result = ogg_stream_pageout(&state.stream, &page);
                    if (!result)
                        break;

                    WriteVorbisPage(state.file, page);
                    if (ogg_page_eos(&page))
                        eos = true;
                }
            }
            Check(error == 0, "OGG:355");
        }

        if (samples_this_run == 0) {
            // The zero-length write signals end-of-stream to libvorbis.
            // The block/page drain above emits the final page.
            break;
        }
        position += samples_this_run;
    }

    Check(std::fflush(state.file) == 0, "Unable to close Ogg Vorbis file");
    Check(std::fclose(state.file) == 0, "Unable to close Ogg Vorbis file");
    state.file = nullptr;
    return 0;
}

class OggPacket {
public:
    explicit OggPacket(int64_t packet_number, bool can_resize = false)
        : resizable_(can_resize)
    {
        packet.packetno = packet_number;
    }

    OggPacket(int64_t packet_number, long size, bool can_resize)
        : OggPacket(packet_number, can_resize)
    {
        Resize(size);
    }

    void Resize(long size)
    {
        Check(size >= 0, "Invalid Ogg packet size");
        buffer.resize(static_cast<size_t>(size));
        packet.packet = buffer.data();
    }

    void MarkBOS() noexcept { packet.b_o_s = 1; }
    void MarkEOS() noexcept { packet.e_o_s = 1; }

    void Write(const void *data, long length)
    {
        Check(length >= 0, "Invalid Ogg packet write size");
        const long next_pos = packet.bytes + length;
        if (static_cast<size_t>(next_pos) > buffer.size()) {
            if (resizable_)
                Resize(static_cast<long>(std::max<size_t>(1024, buffer.size() * 2)));
            else
                throw std::runtime_error("Buffer overflow in OGG packet");
        }
        Check(static_cast<size_t>(next_pos) <= buffer.size(),
              "Buffer overflow in OGG packet");
        if (length != 0)
            std::memcpy(buffer.data() + packet.bytes, data, static_cast<size_t>(length));
        packet.bytes = next_pos;
    }

    template<typename T>
    void WriteInteger(T value)
    {
        static_assert(std::is_integral<T>::value, "Integral values only");
        unsigned char bytes[sizeof(T)];
        using Unsigned = typename std::make_unsigned<T>::type;
        const Unsigned raw = static_cast<Unsigned>(value);
        for (size_t i = 0; i < sizeof(T); ++i)
            bytes[i] = static_cast<unsigned char>((raw >> (i * 8)) & 0xff);
        Write(bytes, static_cast<long>(sizeof(T)));
    }

    unsigned char *GetBuffer() { return buffer.data(); }
    size_t GetBufferSize() const { return buffer.size(); }

    ogg_packet packet{};

private:
    std::vector<unsigned char> buffer;
    bool resizable_{false};
};

struct OpusState {
    OpusMSEncoder *encoder{nullptr};
    ~OpusState()
    {
        if (encoder != nullptr)
            opus_multistream_encoder_destroy(encoder);
    }
};

struct OggState {
    ogg_stream_state stream{};
    OggPacket audio_packet{2};

    OggState()
    {
        std::mt19937 gen(static_cast<uint32_t>(std::time(nullptr)));
        ogg_stream_init(&stream, static_cast<int>(gen()));
    }

    ~OggState() { ogg_stream_clear(&stream); }

    void PacketIn(const OggPacket &packet)
    {
        ogg_stream_packetin(
            &stream, const_cast<ogg_packet *>(&packet.packet));
    }

    void WritePage(FILE *file, const ogg_page &page)
    {
        CheckOggWrite(file, page.header, static_cast<size_t>(page.header_len));
        CheckOggWrite(file, page.body, static_cast<size_t>(page.body_len));
    }

    void WriteOut(FILE *file)
    {
        ogg_page page{};
        while (ogg_stream_pageout(&stream, &page))
            WritePage(file, page);
    }

    void Flush(FILE *file)
    {
        ogg_page page{};
        while (ogg_stream_flush(&stream, &page))
            WritePage(file, page);
    }
};

int32_t GetBestFrameSize(int32_t samples_count, int32_t sample_rate)
{
    static const int32_t multipliers[] = {25, 50, 100, 200, 400, 600};
    for (const auto multiplier : multipliers) {
        const auto frame_size = multiplier * sample_rate / 10000;
        if (samples_count <= frame_size)
            return frame_size;
    }
    return 60 * sample_rate / 1000;
}

void WriteOpusHeader(
    OggState &ogg,
    FILE *file,
    uint32_t sample_rate,
    uint32_t channels,
    uint16_t preskip,
    uint8_t channel_mapping,
    uint8_t stream_count,
    uint8_t coupled_count,
    const uint8_t *stream_map)
{
    const long header_size =
        8 + 1 + 1 + 2 + 4 + 2 + 1 +
        (channel_mapping == 0 ? 0 : 1 + 1 + static_cast<long>(channels));

    OggPacket header_packet(0, header_size, false);
    header_packet.MarkBOS();
    header_packet.Write("OpusHead", 8);
    header_packet.WriteInteger<uint8_t>(1);
    header_packet.WriteInteger<uint8_t>(static_cast<uint8_t>(channels));
    header_packet.WriteInteger<uint16_t>(preskip);
    header_packet.WriteInteger<uint32_t>(sample_rate);
    header_packet.WriteInteger<uint16_t>(0);
    header_packet.WriteInteger<uint8_t>(channel_mapping);

    if (channel_mapping > 0) {
        header_packet.WriteInteger<uint8_t>(stream_count);
        header_packet.WriteInteger<uint8_t>(coupled_count);
        for (uint32_t i = 0; i < channels; ++i)
            header_packet.WriteInteger<uint8_t>(stream_map[i]);
    }

    assert(header_packet.packet.bytes == header_size);
    ogg.PacketIn(header_packet);
    ogg.Flush(file);
}

void WriteOpusTags(
    OggState &ogg,
    FILE *file,
    const AudacityTag *tags,
    size_t tag_count)
{
    OggPacket comments_packet(1, true);
    comments_packet.Write("OpusTags", 8);

    const char *vendor = opus_get_version_string();
    const size_t vendor_size = std::strlen(vendor);
    comments_packet.WriteInteger<uint32_t>(static_cast<uint32_t>(vendor_size));
    comments_packet.Write(vendor, static_cast<long>(vendor_size));
    comments_packet.WriteInteger<uint32_t>(static_cast<uint32_t>(tag_count));

    for (size_t i = 0; i < tag_count; ++i) {
        const char *original_key = tags[i].name == nullptr ? "" : tags[i].name;
        const char *key = IsYearTag(original_key) ? "DATE" : original_key;
        const char *value = tags[i].value == nullptr ? "" : tags[i].value;
        const size_t key_size = std::strlen(key);
        const size_t value_size = std::strlen(value);
        const uint64_t comment_size =
            static_cast<uint64_t>(key_size) + value_size + 1;

        Check(comment_size <= std::numeric_limits<uint32_t>::max(),
              "Opus comment is too large");
        comments_packet.WriteInteger<uint32_t>(
            static_cast<uint32_t>(comment_size));
        comments_packet.Write(key, static_cast<long>(key_size));
        comments_packet.Write("=", 1);
        comments_packet.Write(value, static_cast<long>(value_size));
    }

    ogg.PacketIn(comments_packet);
    ogg.Flush(file);
}

int ExportOpus(
    const char *path,
    uint32_t sample_rate,
    uint32_t channels,
    const float *interleaved,
    uint64_t frames,
    const AudacityOpusOptions *options,
    const AudacityTag *tags,
    size_t tag_count)
{
    const int32_t sample_rate_i32 = static_cast<int32_t>(sample_rate);
    const int32_t sample_rate_factor = 48000 / sample_rate_i32;
    const int32_t frame_size =
        options->frame_multiplier * sample_rate_i32 / 10000;
    Check(frame_size > 0, "Invalid Opus frame duration");

    OpusState opus;
    uint8_t channel_mapping = 0;
    uint8_t stream_count = 0;
    uint8_t coupled_count = 0;
    uint8_t stream_map[255]{};
    int error = OPUS_OK;

    if (channels <= 2) {
        channel_mapping = 0;
        stream_count = 1;
        coupled_count = static_cast<uint8_t>(channels - 1);
        stream_map[0] = 0;
        stream_map[1] = 1;
        opus.encoder = opus_multistream_encoder_create(
            sample_rate_i32,
            static_cast<int>(channels),
            stream_count,
            coupled_count,
            stream_map,
            options->application,
            &error);
    } else {
        channel_mapping = channels <= 8 ? 1 : 255;
        int nb_streams = 0;
        int nb_coupled = 0;
        opus.encoder = opus_multistream_surround_encoder_create(
            sample_rate_i32,
            static_cast<int>(channels),
            channel_mapping,
            &nb_streams,
            &nb_coupled,
            stream_map,
            options->application,
            &error);
        stream_count = static_cast<uint8_t>(nb_streams);
        coupled_count = static_cast<uint8_t>(nb_coupled);
    }

    Check(error == OPUS_OK && opus.encoder != nullptr,
          "Unable to create Opus encoder");

    error = opus_multistream_encoder_ctl(
        opus.encoder, OPUS_SET_BITRATE(options->bitrate));
    Check(error == OPUS_OK, "Unable to set bitrate");

    error = opus_multistream_encoder_ctl(
        opus.encoder, OPUS_SET_COMPLEXITY(std::clamp(options->quality, 0, 10)));
    Check(error == OPUS_OK, "Unable to set complexity");

    error = opus_multistream_encoder_ctl(
        opus.encoder, OPUS_SET_BANDWIDTH(options->cutoff));
    Check(error == OPUS_OK, "Unable to set bandwidth");

    error = opus_multistream_encoder_ctl(
        opus.encoder, OPUS_SET_VBR(options->vbr_mode == 0 ? 0 : 1));
    Check(error == OPUS_OK, "Unable to set VBR mode");

    if (options->vbr_mode == 2) {
        error = opus_multistream_encoder_ctl(
            opus.encoder, OPUS_SET_VBR_CONSTRAINT(1));
        Check(error == OPUS_OK, "Unable to set CVBR mode");
    }

    int lookahead = 0;
    error = opus_multistream_encoder_ctl(
        opus.encoder, OPUS_GET_LOOKAHEAD(&lookahead));
    Check(error == OPUS_OK, "Unable to get lookahead");

    const int64_t calculated_preskip =
        static_cast<int64_t>(lookahead) * sample_rate_factor;
    Check(calculated_preskip >= 0 &&
              calculated_preskip < std::numeric_limits<uint16_t>::max(),
          "Failed to calculate correct preskip");
    const uint16_t preskip = static_cast<uint16_t>(calculated_preskip);

    OggState ogg;
    ogg.audio_packet.Resize(
        static_cast<long>(frame_size * sizeof(float) * channels));

    FILE *file = std::fopen(path, "wb");
    Check(file != nullptr, "Unable to open target file for writing");

    try {
        WriteOpusHeader(
            ogg, file, sample_rate, channels, preskip, channel_mapping,
            stream_count, coupled_count, stream_map);
        WriteOpusTags(ogg, file, tags, tag_count);

        int64_t granule_position = 0;
        int32_t latency_left = preskip;
        uint64_t position = 0;
        std::vector<float> encode_buffer;

        while (position < frames) {
            const auto samples_this_run = static_cast<int32_t>(
                std::min<uint64_t>(static_cast<uint64_t>(frame_size), frames - position));
            const auto best_frame_size =
                GetBestFrameSize(samples_this_run, sample_rate_i32);

            const float *mixed_audio_buffer =
                interleaved + position * channels;
            int32_t submitted_samples = samples_this_run;

            if (samples_this_run < best_frame_size) {
                encode_buffer.resize(
                    static_cast<size_t>(best_frame_size) * channels);
                std::copy(
                    mixed_audio_buffer,
                    mixed_audio_buffer +
                        static_cast<size_t>(samples_this_run) * channels,
                    encode_buffer.begin());
                std::fill(
                    encode_buffer.begin() +
                        static_cast<size_t>(samples_this_run) * channels,
                    encode_buffer.begin() +
                        static_cast<size_t>(best_frame_size) * channels,
                    0.0f);
                mixed_audio_buffer = encode_buffer.data();

                const int32_t zeroes_count =
                    best_frame_size - samples_this_run;
                if (zeroes_count < latency_left)
                    submitted_samples += zeroes_count;
                else
                    submitted_samples += latency_left;
                latency_left = std::max(0, latency_left - zeroes_count);
            }

            const int result = opus_multistream_encode_float(
                opus.encoder,
                mixed_audio_buffer,
                best_frame_size,
                ogg.audio_packet.GetBuffer(),
                static_cast<opus_int32>(ogg.audio_packet.GetBufferSize()));
            Check(result >= 0, "Failed to encode input buffer");

            granule_position +=
                static_cast<int64_t>(submitted_samples) * sample_rate_factor;
            ogg.audio_packet.packet.bytes = result;
            ogg.audio_packet.packet.granulepos = granule_position;
            if (latency_left == 0)
                ogg.audio_packet.MarkEOS();

            ogg.PacketIn(ogg.audio_packet);
            ogg.WriteOut(file);
            ogg.audio_packet.packet.packetno++;
            position += static_cast<uint64_t>(samples_this_run);
        }

        // Flush the encoder's preskip latency, matching Audacity's packet and
        // granule-position updates.
        while (latency_left > 0) {
            const int32_t flush_frame_size =
                GetBestFrameSize(latency_left, sample_rate_i32);
            encode_buffer.resize(
                static_cast<size_t>(flush_frame_size) * channels);
            std::fill(encode_buffer.begin(), encode_buffer.end(), 0.0f);

            const int32_t samples_out =
                std::min(latency_left, flush_frame_size);
            const int result = opus_multistream_encode_float(
                opus.encoder,
                encode_buffer.data(),
                flush_frame_size,
                ogg.audio_packet.GetBuffer(),
                static_cast<opus_int32>(ogg.audio_packet.GetBufferSize()));
            Check(result >= 0, "Failed to encode input buffer");

            granule_position +=
                static_cast<int64_t>(samples_out) * sample_rate_factor;
            ogg.audio_packet.packet.bytes = result;
            ogg.audio_packet.packet.granulepos = granule_position;
            if (latency_left == samples_out)
                ogg.audio_packet.MarkEOS();

            ogg.PacketIn(ogg.audio_packet);
            ogg.WriteOut(file);
            ogg.audio_packet.packet.packetno++;
            latency_left -= samples_out;
        }

        ogg.Flush(file);
        Check(std::fflush(file) == 0, "Unable to write Ogg output");
        Check(std::fclose(file) == 0, "Unable to close Ogg output");
        file = nullptr;
    } catch (...) {
        if (file != nullptr)
            std::fclose(file);
        throw;
    }

    return 0;
}

} // namespace

extern "C" int audacity_export_vorbis(
    const char *path,
    uint32_t sample_rate,
    uint32_t channels,
    const float *interleaved,
    uint64_t frames,
    const AudacityVorbisOptions *options,
    const AudacityTag *tags,
    size_t tag_count,
    char *error_message,
    size_t error_message_size)
{
    try {
        Check(path != nullptr && options != nullptr, "Invalid export arguments");
        Check(channels > 0 && channels <= 255, "Invalid channel count");
        Check(sample_rate > 0, "Unsupported sample rate");
        Check(frames == 0 || interleaved != nullptr, "Missing audio samples");
        Check(tag_count == 0 || tags != nullptr, "Missing tag array");
        return ExportVorbis(
            path, sample_rate, channels, interleaved, frames,
            options, tags, tag_count);
    } catch (const std::exception &error) {
        SetError(error_message, error_message_size, error.what());
        return -1;
    } catch (...) {
        SetError(error_message, error_message_size, "Unknown Vorbis export failure");
        return -1;
    }
}

extern "C" int audacity_export_opus(
    const char *path,
    uint32_t sample_rate,
    uint32_t channels,
    const float *interleaved,
    uint64_t frames,
    const AudacityOpusOptions *options,
    const AudacityTag *tags,
    size_t tag_count,
    char *error_message,
    size_t error_message_size)
{
    try {
        Check(path != nullptr && options != nullptr, "Invalid export arguments");
        Check(channels > 0 && channels <= 255, "Invalid channel count");
        Check(sample_rate == 8000 || sample_rate == 12000 ||
                  sample_rate == 16000 || sample_rate == 24000 ||
                  sample_rate == 48000,
              "Unsupported sample rate");
        Check(frames == 0 || interleaved != nullptr, "Missing audio samples");
        Check(tag_count == 0 || tags != nullptr, "Missing tag array");
        return ExportOpus(
            path, sample_rate, channels, interleaved, frames,
            options, tags, tag_count);
    } catch (const std::exception &error) {
        SetError(error_message, error_message_size, error.what());
        return -1;
    } catch (...) {
        SetError(error_message, error_message_size, "Unknown Opus export failure");
        return -1;
    }
}
