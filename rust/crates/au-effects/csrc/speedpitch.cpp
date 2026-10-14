#include <soundtouch/SoundTouch.h>

extern "C" {

void* aust_new()
{
    return new soundtouch::SoundTouch();
}

void aust_free(void* instance)
{
    delete static_cast<soundtouch::SoundTouch*>(instance);
}

void aust_set_sample_rate(void* instance, unsigned sample_rate)
{
    static_cast<soundtouch::SoundTouch*>(instance)->setSampleRate(sample_rate);
}

void aust_set_channels(void* instance, unsigned channels)
{
    static_cast<soundtouch::SoundTouch*>(instance)->setChannels(channels);
}

void aust_set_tempo_change(void* instance, double percent)
{
    static_cast<soundtouch::SoundTouch*>(instance)->setTempoChange(percent);
}

void aust_set_pitch_semitones(void* instance, double semitones)
{
    static_cast<soundtouch::SoundTouch*>(instance)->setPitchSemiTones(semitones);
}

void aust_set_rate_change(void* instance, double percent)
{
    static_cast<soundtouch::SoundTouch*>(instance)->setRateChange(percent);
}

int aust_set_setting(void* instance, int setting, int value)
{
    return static_cast<soundtouch::SoundTouch*>(instance)->setSetting(setting, value)
        ? 1
        : 0;
}

void aust_put_samples(void* instance, const float* samples, unsigned frames)
{
    static_cast<soundtouch::SoundTouch*>(instance)->putSamples(samples, frames);
}

unsigned aust_receive_samples(void* instance, float* samples, unsigned frames)
{
    return static_cast<soundtouch::SoundTouch*>(instance)->receiveSamples(samples, frames);
}

void aust_flush(void* instance)
{
    static_cast<soundtouch::SoundTouch*>(instance)->flush();
}

unsigned aust_num_samples(void* instance)
{
    return static_cast<soundtouch::SoundTouch*>(instance)->numSamples();
}

}
