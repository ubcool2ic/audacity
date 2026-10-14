#include <functional>
// noise_reduction_ref.cpp
// Standalone Audacity 3.7.3 Noise Reduction reference runner.
// Build with:
// g++ -O1 -std=c++17 noise_reduction_ref.cpp -L/usr/lib/audacity \
//   -l:lib-fft.so -Wl,-rpath,/usr/lib/audacity

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <limits>
#include <memory>
#include <string>
#include <vector>

// Audacity's sampleCount is a small value type. Its in-memory representation
// is one signed 64-bit sample count.
class sampleCount {
public:
   constexpr sampleCount(std::int64_t value = 0) noexcept : mValue{value} {}
   constexpr double as_double() const noexcept
   {
      return static_cast<double>(mValue);
   }
   constexpr std::size_t as_size_t() const noexcept
   {
      return static_cast<std::size_t>(mValue);
   }
   constexpr operator std::int64_t() const noexcept { return mValue; }

   sampleCount& operator+=(std::int64_t value) noexcept
   {
      mValue += value;
      return *this;
   }
   sampleCount& operator-=(std::int64_t value) noexcept
   {
      mValue -= value;
      return *this;
   }
   sampleCount& operator++() noexcept
   {
      ++mValue;
      return *this;
   }

private:
   std::int64_t mValue;
};

// Declarations matching the Audacity FFT library ABI.
enum eWindowFunctions : int {
   eWinFuncRectangular,
   eWinFuncBartlett,
   eWinFuncHamming,
   eWinFuncHann,
   eWinFuncBlackman,
   eWinFuncBlackmanHarris,
   eWinFuncWelch,
   eWinFuncGaussian25,
   eWinFuncGaussian35,
   eWinFuncGaussian45,
   eWinFuncCount
};

using fft_type = float;

struct FFTParam {
   std::unique_ptr<int[]> BitReversed;
   std::unique_ptr<fft_type[]> SinTable;
   std::size_t Points;
};

struct FFTDeleter {
   void operator()(FFTParam* p) const;
};

using HFFT = std::unique_ptr<FFTParam, FFTDeleter>;

HFFT GetFFT(std::size_t);
void RealFFTf(fft_type*, const FFTParam*);
void InverseRealFFTf(fft_type*, const FFTParam*);
void NewWindowFunc(int, std::size_t, bool, float*);

// This declaration preserves SpectrumTransformer's data-member order,
// virtual-function order, and public ABI as declared in SpectrumTransformer.h.
class SpectrumTransformer {
public:
   using FloatVector = std::vector<float>;
   using WindowProcessor = std::function<bool(SpectrumTransformer&)>;

   SpectrumTransformer(
      bool needsOutput, eWindowFunctions inWindowType,
      eWindowFunctions outWindowType, std::size_t windowSize,
      unsigned stepsPerWindow, bool leadingPadding, bool trailingPadding);
   virtual ~SpectrumTransformer();

   bool NeedsOutput() const { return mNeedsOutput; }
   bool Start(std::size_t queueLength);
   bool ProcessSamples(const WindowProcessor& processor, const float* buffer,
                       std::size_t len);
   bool Finish(const WindowProcessor& processor);

   struct Window {
      explicit Window(std::size_t windowSize)
         : mRealFFTs(windowSize / 2), mImagFFTs(windowSize / 2)
      {
      }
      virtual ~Window();

      void Zero()
      {
         const auto size = mRealFFTs.size();
         auto pFill = mRealFFTs.data();
         std::fill(pFill, pFill + size, 0.0f);
         pFill = mImagFFTs.data();
         std::fill(pFill, pFill + size, 0.0f);
      }

      FloatVector mRealFFTs;
      FloatVector mImagFFTs;
   };

   virtual std::unique_ptr<Window> NewWindow(std::size_t windowSize);
   virtual bool DoStart();
   virtual void DoOutput(const float* outBuffer, std::size_t stepSize) = 0;
   virtual bool DoFinish();

   std::size_t TotalQueueSize() const { return mQueue.size(); }
   std::size_t CurrentQueueSize() const;
   bool QueueIsFull() const;
   Window& Nth(int n) { return *mQueue[n]; }
   Window& Newest() { return **mQueue.begin(); }
   Window& Latest() { return **mQueue.rbegin(); }

private:
   void ResizeQueue(std::size_t queueLength);
   void FillFirstWindow();
   void RotateWindows();
   void OutputStep();

protected:
   const std::size_t mWindowSize;
   const std::size_t mSpectrumSize;
   const unsigned mStepsPerWindow;
   const std::size_t mStepSize;
   const bool mLeadingPadding;
   const bool mTrailingPadding;

private:
   std::vector<std::unique_ptr<Window>> mQueue;
   HFFT hFFT;
   sampleCount mInSampleCount = 0;
   sampleCount mOutStepCount = 0;
   std::size_t mInWavePos = 0;
   FloatVector mFFTBuffer;
   FloatVector mInWaveBuffer;
   FloatVector mOutOverlapBuffer;
   FloatVector mInWindow;
   FloatVector mOutWindow;
   const bool mNeedsOutput;
};

using FloatVector = std::vector<float>;

enum NoiseReductionChoice {
   NRC_REDUCE_NOISE,
   NRC_ISOLATE_NOISE,
   NRC_LEAVE_RESIDUE,
};

enum DiscriminationMethod : std::size_t {
   DM_MEDIAN,
   DM_SECOND_GREATEST,
   DM_OLD_METHOD,
   DM_N_METHODS,
   DM_DEFAULT_METHOD = DM_SECOND_GREATEST,
};

enum WindowTypes : unsigned {
   WT_RECTANGULAR_HANN = 0,
   WT_HANN_RECTANGULAR,
   WT_HANN_HANN,
   WT_BLACKMAN_HANN,
   WT_HAMMING_RECTANGULAR,
   WT_HAMMING_HANN,
   WT_N_WINDOW_TYPES,
   WT_DEFAULT_WINDOW_TYPES = WT_HANN_HANN
};

static constexpr float minSignalTime = 0.05f;
static constexpr int DEFAULT_WINDOW_SIZE_CHOICE = 8;
static constexpr int DEFAULT_STEPS_PER_WINDOW_CHOICE = 1;

class NoiseReductionBase {
public:
   class Settings {
   public:
      Settings()
         : mDoProfile{true},
           mNewSensitivity{6.0},
           mFreqSmoothingBands{6.0},
           mNoiseGain{6.0},
           mAttackTime{0.02},
           mReleaseTime{0.10},
           mOldSensitivity{0.0},
           mNoiseReductionChoice{NRC_REDUCE_NOISE},
           mWindowTypes{WT_DEFAULT_WINDOW_TYPES},
           mWindowSizeChoice{DEFAULT_WINDOW_SIZE_CHOICE},
           mStepsPerWindowChoice{DEFAULT_STEPS_PER_WINDOW_CHOICE},
           mMethod{DM_DEFAULT_METHOD}
      {
      }

      std::size_t WindowSize() const
      {
         return 1u << (3 + mWindowSizeChoice);
      }
      unsigned StepsPerWindow() const
      {
         return 1u << (1 + mStepsPerWindowChoice);
      }
      std::size_t SpectrumSize() const
      {
         return 1 + WindowSize() / 2;
      }
      std::size_t StepSize() const
      {
         return WindowSize() / StepsPerWindow();
      }

      bool mDoProfile;
      double mNewSensitivity;
      double mFreqSmoothingBands;
      double mNoiseGain;
      double mAttackTime;
      double mReleaseTime;
      double mOldSensitivity;
      int mNoiseReductionChoice;
      int mWindowTypes;
      int mWindowSizeChoice;
      int mStepsPerWindowChoice;
      int mMethod;
   };

   class Statistics {
   public:
      Statistics(std::size_t spectrumSize, double rate, int windowTypes)
         : mRate{rate},
           mWindowSize{(spectrumSize - 1) * 2},
           mWindowTypes{windowTypes},
           mTotalWindows{0},
           mTrackWindows{0},
           mSums(spectrumSize),
           mMeans(spectrumSize)
      {
      }

      double mRate;
      std::size_t mWindowSize;
      int mWindowTypes;
      unsigned mTotalWindows;
      unsigned mTrackWindows;
      FloatVector mSums;
      FloatVector mMeans;
   };

   class Worker;

   // Stubbed progress callback: never cancels.
   bool TrackProgress(unsigned, double) { return false; }
};

class TrackSpectrumTransformer : public SpectrumTransformer {
public:
   TrackSpectrumTransformer(
      std::vector<float>* outputTrack, bool needsOutput,
      eWindowFunctions inWindowType, eWindowFunctions outWindowType,
      std::size_t windowSize, unsigned stepsPerWindow,
      bool leadingPadding, bool trailingPadding)
      : SpectrumTransformer{needsOutput, inWindowType, outWindowType,
                            windowSize, stepsPerWindow, leadingPadding,
                            trailingPadding},
        mOutputTrack{outputTrack}
   {
   }

   ~TrackSpectrumTransformer() override = default;

   bool Process(const WindowProcessor& processor,
                const std::vector<float>& channel,
                std::size_t queueLength, sampleCount start, sampleCount len)
   {
      if (!Start(queueLength))
         return false;

      bool bLoopSuccess = true;
      auto samplePos = static_cast<std::size_t>(start);
      const auto end = samplePos + static_cast<std::size_t>(len);
      while (bLoopSuccess && samplePos < end) {
         const auto blockSize = end - samplePos;
         bLoopSuccess =
            ProcessSamples(processor, channel.data() + samplePos, blockSize);
         samplePos += blockSize;
      }

      if (!Finish(processor))
         return false;

      return bLoopSuccess;
   }

   static bool PostProcess(std::vector<float>& outputTrack, sampleCount len)
   {
      // Audacity flushes the output track and clears any samples past len.
      if (outputTrack.size() > static_cast<std::size_t>(len))
         outputTrack.resize(static_cast<std::size_t>(len));
      return true;
   }

protected:
   bool DoStart() override { return SpectrumTransformer::DoStart(); }

   void DoOutput(const float* outBuffer, std::size_t stepSize) override
   {
      mOutputTrack->insert(mOutputTrack->end(), outBuffer,
                           outBuffer + stepSize);
   }

   bool DoFinish() override { return SpectrumTransformer::DoFinish(); }

private:
   std::vector<float>* const mOutputTrack;
};

class NoiseReductionBase::Worker final {
public:
   using Settings = NoiseReductionBase::Settings;
   using Statistics = NoiseReductionBase::Statistics;

   Worker(NoiseReductionBase& effect, const Settings& settings,
          Statistics& statistics);
   ~Worker();

   bool Process(eWindowFunctions inWindowType, eWindowFunctions outWindowType,
                const std::vector<std::vector<float>>& tracks,
                std::vector<std::vector<float>>* outputs, double rate);

   static bool Processor(SpectrumTransformer& transformer);

   void ApplyFreqSmoothing(FloatVector& gains);
   void GatherStatistics(class MyTransformer& transformer);
   bool Classify(class MyTransformer& transformer, unsigned nWindows, int band);
   void ReduceNoise(class MyTransformer& transformer);
   void FinishTrackStatistics();

   const bool mDoProfile;
   NoiseReductionBase& mEffect;
   const Settings& mSettings;
   Statistics& mStatistics;
   FloatVector mFreqSmoothingScratch;
   const std::size_t mFreqSmoothingBins;
   std::size_t mBinLow;
   std::size_t mBinHigh;
   const int mNoiseReductionChoice;
   const int mMethod;
   const double mNewSensitivity;
   float mOneBlockAttack;
   float mOneBlockRelease;
   float mNoiseAttenFactor;
   float mOldSensitivityFactor;
   unsigned mNWindowsToExamine;
   unsigned mCenter;
   unsigned mHistoryLen;
   unsigned mProgressTrackCount = 0;
   sampleCount mLen = 0;
   sampleCount mProgressWindowCount = 0;
};

class MyTransformer : public TrackSpectrumTransformer {
public:
   MyTransformer(NoiseReductionBase::Worker& worker,
                 std::vector<float>* outputTrack, bool needsOutput,
                 eWindowFunctions inWindowType, eWindowFunctions outWindowType,
                 std::size_t windowSize, unsigned stepsPerWindow,
                 bool leadingPadding, bool trailingPadding)
      : TrackSpectrumTransformer{outputTrack, needsOutput, inWindowType,
                                 outWindowType, windowSize, stepsPerWindow,
                                 leadingPadding, trailingPadding},
        mWorker{worker}
   {
   }

   struct MyWindow : public Window {
      explicit MyWindow(std::size_t windowSize)
         : Window{windowSize},
           mSpectrums(windowSize / 2 + 1),
           mGains(windowSize / 2 + 1)
      {
      }
      ~MyWindow() override;

      FloatVector mSpectrums;
      FloatVector mGains;
   };

   MyWindow& NthWindow(int nn)
   {
      return static_cast<MyWindow&>(Nth(nn));
   }

   std::unique_ptr<Window> NewWindow(std::size_t windowSize) override;
   bool DoStart() override;
   bool DoFinish() override;

   NoiseReductionBase::Worker& mWorker;
};

std::unique_ptr<SpectrumTransformer::Window>
MyTransformer::NewWindow(std::size_t windowSize)
{
   return std::make_unique<MyWindow>(windowSize);
}

MyTransformer::MyWindow::~MyWindow() = default;

NoiseReductionBase::Worker::Worker(NoiseReductionBase& effect,
                                   const Settings& settings,
                                   Statistics& statistics)
   : mDoProfile{settings.mDoProfile},
     mEffect{effect},
     mSettings{settings},
     mStatistics{statistics},
     mFreqSmoothingScratch(mSettings.SpectrumSize()),
     mFreqSmoothingBins{
        static_cast<std::size_t>(std::max(0.0, settings.mFreqSmoothingBands))},
     mBinLow{0},
     mBinHigh{mSettings.SpectrumSize()},
     mNoiseReductionChoice{settings.mNoiseReductionChoice},
     mMethod{settings.mMethod},
     mNewSensitivity{settings.mNewSensitivity * std::log(10.0)}
{
   const auto sampleRate = mStatistics.mRate;
   const double noiseGain = -settings.mNoiseGain;
   const unsigned nAttackBlocks =
      1 + static_cast<int>(settings.mAttackTime * sampleRate /
                           mSettings.StepSize());
   const unsigned nReleaseBlocks =
      1 + static_cast<int>(settings.mReleaseTime * sampleRate /
                           mSettings.StepSize());

   // DB_TO_LINEAR for amplitude gains.
   mNoiseAttenFactor = static_cast<float>(std::pow(10.0, noiseGain / 20.0));
   mOneBlockAttack =
      static_cast<float>(std::pow(10.0, noiseGain / nAttackBlocks / 20.0));
   mOneBlockRelease =
      static_cast<float>(std::pow(10.0, noiseGain / nReleaseBlocks / 20.0));
   mOldSensitivityFactor =
      static_cast<float>(std::pow(10.0, settings.mOldSensitivity / 10.0));

   mNWindowsToExamine =
      (mMethod == DM_OLD_METHOD)
         ? static_cast<unsigned>(
              std::max(2, static_cast<int>(minSignalTime * sampleRate /
                                           mSettings.StepSize())))
         : 1 + mSettings.StepsPerWindow();

   mCenter = mNWindowsToExamine / 2;

   if (mDoProfile)
      mHistoryLen = 1;
   else
      mHistoryLen =
         std::max(mNWindowsToExamine,
                  mCenter + nAttackBlocks);
}

NoiseReductionBase::Worker::~Worker() = default;

bool MyTransformer::DoStart()
{
   for (std::size_t ii = 0, nn = TotalQueueSize(); ii < nn; ++ii) {
      MyWindow& record = NthWindow(static_cast<int>(ii));
      std::fill(record.mSpectrums.begin(), record.mSpectrums.end(), 0.0);
      std::fill(record.mGains.begin(), record.mGains.end(),
                mWorker.mNoiseAttenFactor);
   }
   return TrackSpectrumTransformer::DoStart();
}

bool MyTransformer::DoFinish()
{
   if (mWorker.mDoProfile)
      mWorker.FinishTrackStatistics();
   return TrackSpectrumTransformer::DoFinish();
}

bool NoiseReductionBase::Worker::Processor(SpectrumTransformer& trans)
{
   auto& transformer = static_cast<MyTransformer&>(trans);
   auto& worker = transformer.mWorker;

   // Compute power spectrum in the newest window.
   {
      auto& record = transformer.NthWindow(0);
      float* pSpectrum = &record.mSpectrums[0];
      const double dc = record.mRealFFTs[0];
      *pSpectrum++ = dc * dc;
      float* pReal = &record.mRealFFTs[1];
      float* pImag = &record.mImagFFTs[1];
      for (std::size_t nn = worker.mSettings.SpectrumSize() - 2; nn--;) {
         const double re = *pReal++;
         const double im = *pImag++;
         *pSpectrum++ = re * re + im * im;
      }
      const double nyquist = record.mImagFFTs[0];
      *pSpectrum = nyquist * nyquist;
   }

   if (worker.mDoProfile)
      worker.GatherStatistics(transformer);
   else
      worker.ReduceNoise(transformer);

   return !worker.mEffect.TrackProgress(
      worker.mProgressTrackCount,
      std::min(1.0, (++worker.mProgressWindowCount).as_double() *
                        worker.mSettings.StepSize() /
                        worker.mLen.as_double()));
}

void NoiseReductionBase::Worker::FinishTrackStatistics()
{
   const auto windows = mStatistics.mTrackWindows;

   // Combine averages in case of multiple profile channels.
   if (windows) {
      const auto multiplier = mStatistics.mTotalWindows;
      const auto denom = windows + multiplier;
      for (std::size_t ii = 0, nn = mStatistics.mMeans.size(); ii < nn; ++ii) {
         auto& mean = mStatistics.mMeans[ii];
         auto& sum = mStatistics.mSums[ii];
         mean = (mean * multiplier + sum) / denom;
         sum = 0;
      }
      mStatistics.mTrackWindows = 0;
      mStatistics.mTotalWindows = denom;
   }
}

void NoiseReductionBase::Worker::GatherStatistics(MyTransformer& transformer)
{
   ++mStatistics.mTrackWindows;

   auto pPower = transformer.NthWindow(0).mSpectrums.data();
   auto pSum = mStatistics.mSums.data();
   for (std::size_t jj = 0; jj < mSettings.SpectrumSize(); ++jj)
      *pSum++ += *pPower++;
}

bool NoiseReductionBase::Worker::Classify(MyTransformer& transformer,
                                          unsigned nWindows, int band)
{
   switch (mMethod) {
   case DM_MEDIAN:
      if (nWindows <= 3)
         goto secondGreatest;
      else if (nWindows <= 5) {
         float greatest = 0.0f;
         float second = 0.0f;
         float third = 0.0f;
         for (unsigned ii = 0; ii < nWindows; ++ii) {
            const float power =
               transformer.NthWindow(static_cast<int>(ii)).mSpectrums[band];
            if (power >= greatest)
               third = second, second = greatest, greatest = power;
            else if (power >= second)
               third = second, second = power;
            else if (power >= third)
               third = power;
         }
         return third <= mNewSensitivity * mStatistics.mMeans[band];
      } else {
         return true;
      }

   secondGreatest:
   case DM_SECOND_GREATEST: {
      float greatest = 0.0f;
      float second = 0.0f;
      for (unsigned ii = 0; ii < nWindows; ++ii) {
         const float power =
            transformer.NthWindow(static_cast<int>(ii)).mSpectrums[band];
         if (power >= greatest)
            second = greatest, greatest = power;
         else if (power >= second)
            second = power;
      }
      return second <= mNewSensitivity * mStatistics.mMeans[band];
   }

   default:
      return true;
   }
}

void NoiseReductionBase::Worker::ApplyFreqSmoothing(FloatVector& gains)
{
   if (mFreqSmoothingBins == 0)
      return;

   const auto spectrumSize = mSettings.SpectrumSize();

   {
      auto pScratch = mFreqSmoothingScratch.data();
      std::fill(pScratch, pScratch + spectrumSize, 0.0f);
   }

   for (std::size_t ii = 0; ii < spectrumSize; ++ii)
      gains[ii] = std::log(gains[ii]);

   for (int ii = 0; ii < static_cast<int>(spectrumSize); ++ii) {
      const int j0 = std::max(0, ii - static_cast<int>(mFreqSmoothingBins));
      const int j1 = std::min(static_cast<int>(spectrumSize - 1),
                              ii + static_cast<int>(mFreqSmoothingBins));
      for (int jj = j0; jj <= j1; ++jj)
         mFreqSmoothingScratch[ii] += gains[jj];
      mFreqSmoothingScratch[ii] /= (j1 - j0 + 1);
   }

   for (std::size_t ii = 0; ii < spectrumSize; ++ii)
      gains[ii] = std::exp(mFreqSmoothingScratch[ii]);
}

void NoiseReductionBase::Worker::ReduceNoise(MyTransformer& transformer)
{
   const auto historyLen = transformer.CurrentQueueSize();
   const auto nWindows =
      std::min<unsigned>(mNWindowsToExamine, static_cast<unsigned>(historyLen));
   const auto spectrumSize = mSettings.SpectrumSize();

   if (mNoiseReductionChoice != NRC_ISOLATE_NOISE) {
      auto& record = transformer.NthWindow(0);
      std::fill(record.mGains.begin(), record.mGains.end(),
                mNoiseAttenFactor);
   }

   if (nWindows > mCenter) {
      auto pGain =
         transformer.NthWindow(static_cast<int>(mCenter)).mGains.data();
      if (mNoiseReductionChoice == NRC_ISOLATE_NOISE) {
         std::fill(pGain, pGain + mBinLow, 0.0f);
         std::fill(pGain + mBinHigh, pGain + spectrumSize, 0.0f);
         pGain += mBinLow;
         for (std::size_t jj = mBinLow; jj < mBinHigh; ++jj) {
            const bool isNoise =
               Classify(transformer, nWindows, static_cast<int>(jj));
            *pGain++ = isNoise ? 1.0f : 0.0f;
         }
      } else {
         std::fill(pGain, pGain + mBinLow, 1.0f);
         std::fill(pGain + mBinHigh, pGain + spectrumSize, 1.0f);
         pGain += mBinLow;
         for (std::size_t jj = mBinLow; jj < mBinHigh; ++jj) {
            const bool isNoise =
               Classify(transformer, nWindows, static_cast<int>(jj));
            if (!isNoise)
               *pGain = 1.0f;
            ++pGain;
         }
      }
   }

   if (mNoiseReductionChoice != NRC_ISOLATE_NOISE) {
      // Attack moves backward in time through the queue.
      for (std::size_t jj = 0; jj < spectrumSize; ++jj) {
         for (unsigned ii = mCenter + 1; ii < historyLen; ++ii) {
            const float minimum =
               std::max(mNoiseAttenFactor,
                        transformer.NthWindow(static_cast<int>(ii - 1))
                              .mGains[jj] *
                           mOneBlockAttack);
            float& gain =
               transformer.NthWindow(static_cast<int>(ii)).mGains[jj];
            if (gain < minimum)
               gain = minimum;
            else
               break;
         }
      }

      // Release moves forward by one window.
      auto pNextGain =
         transformer.NthWindow(static_cast<int>(mCenter - 1)).mGains.data();
      auto pThisGain =
         transformer.NthWindow(static_cast<int>(mCenter)).mGains.data();
      for (auto nn = mSettings.SpectrumSize(); nn--;) {
         *pNextGain =
            std::max(*pNextGain,
                     std::max(mNoiseAttenFactor,
                              *pThisGain++ * mOneBlockRelease));
         ++pNextGain;
      }
   }

   if (transformer.QueueIsFull()) {
      auto& record =
         transformer.NthWindow(static_cast<int>(historyLen - 1));
      const auto last = mSettings.SpectrumSize() - 1;

      if (mNoiseReductionChoice != NRC_ISOLATE_NOISE)
         ApplyFreqSmoothing(record.mGains);

      const float* pGain = &record.mGains[1];
      float* pReal = &record.mRealFFTs[1];
      float* pImag = &record.mImagFFTs[1];
      auto nn = mSettings.SpectrumSize() - 2;

      if (mNoiseReductionChoice == NRC_LEAVE_RESIDUE) {
         for (; nn--;) {
            const double gain = *pGain++ - 1.0;
            *pReal++ *= gain;
            *pImag++ *= gain;
         }
         record.mRealFFTs[0] *= record.mGains[0] - 1.0;
         record.mImagFFTs[0] *= record.mGains[last] - 1.0;
      } else {
         for (; nn--;) {
            const double gain = *pGain++;
            *pReal++ *= gain;
            *pImag++ *= gain;
         }
         record.mRealFFTs[0] *= record.mGains[0];
         record.mImagFFTs[0] *= record.mGains[last];
      }
   }
}

bool NoiseReductionBase::Worker::Process(
   eWindowFunctions inWindowType, eWindowFunctions outWindowType,
   const std::vector<std::vector<float>>& tracks,
   std::vector<std::vector<float>>* outputs, double rate)
{
   mProgressTrackCount = 0;

   for (std::size_t track = 0; track < tracks.size(); ++track) {
      const auto& channel = tracks[track];
      if (rate != mStatistics.mRate)
         return false;

      const auto len = channel.size();
      mProgressWindowCount = 0;
      mLen = static_cast<std::int64_t>(len);
      const auto extra =
         (mSettings.StepsPerWindow() - 1) * mSettings.SpectrumSize();
      if (mDoProfile)
         mLen -= static_cast<std::int64_t>(extra);
      else
         mLen += static_cast<std::int64_t>(extra);

      std::vector<float>* outputTrack =
         outputs ? &(*outputs)[track] : nullptr;
      MyTransformer transformer{
         *this,
         outputTrack,
         !mSettings.mDoProfile,
         inWindowType,
         outWindowType,
         mSettings.WindowSize(),
         mSettings.StepsPerWindow(),
         !mSettings.mDoProfile,
         !mSettings.mDoProfile};

      if (!transformer.Process(Processor, channel, mHistoryLen, 0,
                               static_cast<std::int64_t>(len)))
         return false;
      ++mProgressTrackCount;
      if (outputTrack)
         MyTransformer::PostProcess(*outputTrack, static_cast<std::int64_t>(len));
   }

   if (mDoProfile && mStatistics.mTotalWindows == 0)
      return false;

   return true;
}

static bool ReadFloats(const std::string& path, std::vector<float>& samples)
{
   std::ifstream file(path, std::ios::binary | std::ios::ate);
   if (!file)
      return false;

   const auto end = file.tellg();
   if (end < 0 || static_cast<std::uint64_t>(end) % sizeof(float) != 0)
      return false;

   samples.resize(static_cast<std::size_t>(end) / sizeof(float));
   file.seekg(0);
   if (!samples.empty())
      file.read(reinterpret_cast<char*>(samples.data()),
                static_cast<std::streamsize>(samples.size() * sizeof(float)));
   return static_cast<bool>(file) || samples.empty();
}

static bool ParsePositive(const char* text, unsigned long long& value)
{
   char* end = nullptr;
   const auto parsed = std::strtoull(text, &end, 10);
   if (!text[0] || !end || *end)
      return false;
   value = parsed;
   return true;
}

static void Usage()
{
   std::cerr
      << "usage: noise_reduction_ref noise.f32 input.f32 rate nchannels "
         "reduction_db sensitivity freq_smoothing_bands "
         "noise_reduction_choice(0 reduce,1 isolate,2 residue) "
         "[window_types(2) window_size(2048) steps_per_window(4) method(1)] "
         "> out.f32\n";
}

int main(int argc, char** argv)
{
   if (argc < 9 || argc > 13) {
      Usage();
      return 2;
   }

   unsigned long long rateArg = 0;
   unsigned long long channelsArg = 0;
   if (!ParsePositive(argv[3], rateArg) ||
       !ParsePositive(argv[4], channelsArg) || rateArg == 0 ||
       channelsArg == 0 ||
       channelsArg > static_cast<unsigned long long>(
                         std::numeric_limits<unsigned>::max())) {
      Usage();
      return 2;
   }

   const auto rate = static_cast<unsigned>(rateArg);
   const auto channels = static_cast<std::size_t>(channelsArg);
   const double reductionDb = std::strtod(argv[5], nullptr);
   const double sensitivity = std::strtod(argv[6], nullptr);
   const double smoothing = std::strtod(argv[7], nullptr);
   const int reductionChoice = std::atoi(argv[8]);

   int windowTypes = WT_DEFAULT_WINDOW_TYPES;
   unsigned long long windowSizeArg = 2048;
   unsigned long long stepsArg = 4;
   int method = DM_DEFAULT_METHOD;

   if (argc >= 10)
      windowTypes = std::atoi(argv[9]);
   if (argc >= 11 && !ParsePositive(argv[10], windowSizeArg)) {
      Usage();
      return 2;
   }
   if (argc >= 12 && !ParsePositive(argv[11], stepsArg)) {
      Usage();
      return 2;
   }
   if (argc >= 13)
      method = std::atoi(argv[12]);

   if (reductionChoice < NRC_REDUCE_NOISE ||
       reductionChoice > NRC_LEAVE_RESIDUE ||
       windowTypes < 0 || windowTypes >= WT_N_WINDOW_TYPES ||
       windowSizeArg < 8 ||
       (windowSizeArg & (windowSizeArg - 1)) != 0 ||
       stepsArg == 0 || stepsArg > windowSizeArg ||
       windowSizeArg % stepsArg != 0) {
      Usage();
      return 2;
   }

   unsigned logWindow = 0;
   for (auto n = windowSizeArg; n > 1; n >>= 1)
      ++logWindow;
   if (logWindow < 3 ||
       logWindow - 3 >
          static_cast<unsigned>(std::numeric_limits<int>::max())) {
      Usage();
      return 2;
   }

   unsigned logSteps = 0;
   for (auto n = stepsArg; n > 1; n >>= 1)
      ++logSteps;
   if ((1ull << logSteps) != stepsArg || logSteps < 1 ||
       logSteps - 1 >
          static_cast<unsigned>(std::numeric_limits<int>::max())) {
      Usage();
      return 2;
   }

   // Audacity 3.7.3 does not expose the old discrimination method.
   if (method < DM_MEDIAN || method > DM_SECOND_GREATEST)
      method = DM_DEFAULT_METHOD;

   NoiseReductionBase::Settings settings;
   settings.mDoProfile = true;
   settings.mNewSensitivity = sensitivity;
   settings.mFreqSmoothingBands = smoothing;
   settings.mNoiseGain = reductionDb;
   settings.mNoiseReductionChoice = reductionChoice;
   settings.mWindowTypes = windowTypes;
   settings.mWindowSizeChoice = static_cast<int>(logWindow - 3);
   settings.mStepsPerWindowChoice = static_cast<int>(logSteps - 1);
   settings.mMethod = method;

   const unsigned minSteps =
      (windowTypes == WT_HANN_HANN || windowTypes == WT_BLACKMAN_HANN ||
       windowTypes == WT_HAMMING_HANN)
         ? 4
         : 2;
   if (settings.StepsPerWindow() < minSteps ||
       settings.StepsPerWindow() > settings.WindowSize()) {
      Usage();
      return 2;
   }

   std::vector<float> noiseInterleaved;
   std::vector<float> inputInterleaved;
   if (!ReadFloats(argv[1], noiseInterleaved) ||
       !ReadFloats(argv[2], inputInterleaved) ||
       noiseInterleaved.size() % channels != 0 ||
       inputInterleaved.size() % channels != 0) {
      std::cerr << "could not read raw float32 input files\n";
      return 2;
   }

   const auto noiseFrames = noiseInterleaved.size() / channels;
   const auto inputFrames = inputInterleaved.size() / channels;
   std::vector<std::vector<float>> noise(channels);
   std::vector<std::vector<float>> input(channels);
   std::vector<std::vector<float>> output(channels);

   for (std::size_t ch = 0; ch < channels; ++ch) {
      noise[ch].resize(noiseFrames);
      input[ch].resize(inputFrames);
      for (std::size_t frame = 0; frame < noiseFrames; ++frame)
         noise[ch][frame] = noiseInterleaved[frame * channels + ch];
      for (std::size_t frame = 0; frame < inputFrames; ++frame)
         input[ch][frame] = inputInterleaved[frame * channels + ch];
   }

   NoiseReductionBase effect;
   NoiseReductionBase::Statistics statistics(
      settings.SpectrumSize(), static_cast<double>(rate), settings.mWindowTypes);

   eWindowFunctions inWindowType;
   eWindowFunctions outWindowType;
   switch (settings.mWindowTypes) {
   case WT_RECTANGULAR_HANN:
      inWindowType = eWinFuncRectangular;
      outWindowType = eWinFuncHann;
      break;
   case WT_HANN_RECTANGULAR:
      inWindowType = eWinFuncHann;
      outWindowType = eWinFuncRectangular;
      break;
   case WT_BLACKMAN_HANN:
      inWindowType = eWinFuncBlackman;
      outWindowType = eWinFuncHann;
      break;
   case WT_HAMMING_RECTANGULAR:
      inWindowType = eWinFuncHamming;
      outWindowType = eWinFuncRectangular;
      break;
   case WT_HAMMING_HANN:
      inWindowType = eWinFuncHamming;
      outWindowType = eWinFuncHann;
      break;
   default:
      inWindowType = outWindowType = eWinFuncHann;
      break;
   }

   {
      NoiseReductionBase::Worker worker{effect, settings, statistics};
      if (!worker.Process(inWindowType, outWindowType, noise, nullptr,
                          static_cast<double>(rate))) {
         std::cerr << "selected noise profile is too short or invalid\n";
         return 1;
      }
   }

   settings.mDoProfile = false;

   {
      NoiseReductionBase::Worker worker{effect, settings, statistics};
      if (!worker.Process(inWindowType, outWindowType, input, &output,
                          static_cast<double>(rate))) {
         std::cerr << "noise reduction processing failed\n";
         return 1;
      }
   }

   std::size_t outputFrames = output.empty() ? 0 : output[0].size();
   for (const auto& channel : output)
      outputFrames = std::min(outputFrames, channel.size());

   std::vector<float> outputInterleaved(outputFrames * channels);
   for (std::size_t frame = 0; frame < outputFrames; ++frame)
      for (std::size_t ch = 0; ch < channels; ++ch)
         outputInterleaved[frame * channels + ch] = output[ch][frame];

   if (!outputInterleaved.empty() &&
       std::fwrite(outputInterleaved.data(), sizeof(float),
                   outputInterleaved.size(), stdout) !=
          outputInterleaved.size()) {
      std::cerr << "failed to write output\n";
      return 1;
   }

   return 0;
}
