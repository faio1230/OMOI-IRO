// link-sss-probe: minimal observe-only Link peer for the S07 Start Stop Sync
// investigation. It mirrors what p0-probe `monitor --link` does with the SDK
// (enable, optional start/stop sync, capture app session state, never commit),
// built for either peer type (two executables, because LinkAudio.hpp defines
// LINK_AUDIO, which changes how the whole SDK is compiled):
//   link-sss-probe        ableton::Link       - plain Link, as in the SDK's LinkHut
//   link-sss-probe-audio  ableton::LinkAudio  - what rusty_link 0.4.9's abl_link
//                                               creates (empty peer name, as abl_link)
// Options: --sss (enable start/stop sync), --seconds N (default 30).
// Prints one line per 500 ms: t, peers, tempo, beat, phase(4), is_playing.
//
// It never sets tempo, beat or transport and never commits a session state.

#if defined(USE_LINK_AUDIO)
#include <ableton/LinkAudio.hpp>
#else
#include <ableton/Link.hpp>
#endif

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <thread>

namespace
{

template <typename L>
int run(L& link, const char* kind, bool sss, int seconds)
{
  link.setNumPeersCallback(
    [](std::size_t n) { std::printf("event peers -> %zu\n", n), std::fflush(stdout); });
  link.setTempoCallback(
    [](double bpm) { std::printf("event tempo -> %.2f\n", bpm), std::fflush(stdout); });
  link.setStartStopCallback([](bool p)
    { std::printf("event is_playing -> %s\n", p ? "true" : "false"), std::fflush(stdout); });
  link.enableStartStopSync(sss);
  link.enable(true);
  std::printf("link-sss-probe kind=%s start_stop_sync=%s seconds=%d (observe only)\n",
    kind, link.isStartStopSyncEnabled() ? "on" : "off", seconds);
  std::fflush(stdout);

  const auto t0 = std::chrono::steady_clock::now();
  const auto end = t0 + std::chrono::seconds(seconds);
  while (std::chrono::steady_clock::now() < end)
  {
    std::this_thread::sleep_for(std::chrono::milliseconds(500));
    const auto state = link.captureAppSessionState(); // capture only, never committed
    const auto now = link.clock().micros();
    const double t =
      std::chrono::duration<double>(std::chrono::steady_clock::now() - t0).count();
    std::printf("t=%7.3f peers=%zu tempo=%.2f beat=%.3f phase=%.3f is_playing=%s\n", t,
      link.numPeers(), state.tempo(), state.beatAtTime(now, 4.), state.phaseAtTime(now, 4.),
      state.isPlaying() ? "true" : "false");
    std::fflush(stdout);
  }
  link.enable(false);
  std::printf("done\n");
  return 0;
}

} // namespace

int main(int argc, char** argv)
{
  bool sss = false;
  int seconds = 30;
  for (int i = 1; i < argc; ++i)
  {
    if (std::strcmp(argv[i], "--sss") == 0)
      sss = true;
    else if (std::strcmp(argv[i], "--seconds") == 0 && i + 1 < argc)
      seconds = std::atoi(argv[++i]);
    else
    {
      std::fprintf(stderr, "usage: link-sss-probe[-audio] [--sss] [--seconds N]\n");
      return 2;
    }
  }
#if defined(USE_LINK_AUDIO)
  ableton::LinkAudio link(120., std::string{});
  return run(link, "LinkAudio", sss, seconds);
#else
  ableton::Link link(120.);
  return run(link, "Link", sss, seconds);
#endif
}
