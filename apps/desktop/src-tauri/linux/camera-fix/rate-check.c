#include <gst/gst.h>

static guint frames;

static void
on_handoff (GstElement * sink, GstBuffer * buffer, GstPad * pad, gpointer user_data)
{
  frames++;
}

static GstElement *
make_capsfilter (const gchar * caps_text)
{
  GstElement *filter = gst_element_factory_make ("capsfilter", NULL);
  GstCaps *caps = gst_caps_from_string (caps_text);

  g_object_set (filter, "caps", caps, NULL);
  gst_caps_unref (caps);
  return filter;
}

int
main (int argc, char **argv)
{
  GstElement *pipeline, *source, *convert, *rate, *sink;
  GstClock *clock;

  gst_init (&argc, &argv);
  pipeline = gst_pipeline_new (NULL);
  source = gst_element_factory_make ("videotestsrc", NULL);
  convert = gst_element_factory_make ("videoconvert", NULL);
  rate = gst_element_factory_make ("videorate", "videorate");
  sink = gst_element_factory_make ("fakesink", NULL);
  if (source == NULL || convert == NULL || rate == NULL || sink == NULL) {
    g_printerr ("missing GStreamer elements\n");
    return 2;
  }
  g_object_set (source, "is-live", TRUE, "do-timestamp", TRUE, NULL);
  g_object_set (sink, "signal-handoffs", TRUE, "sync", FALSE, NULL);
  g_signal_connect (sink, "handoff", G_CALLBACK (on_handoff), NULL);

  GstElement *source_caps = make_capsfilter ("video/x-raw,width=64,height=64,framerate=30/1");
  GstElement *rate_caps = make_capsfilter ("video/x-raw,framerate=15/1");
  gst_bin_add_many (GST_BIN (pipeline), source, source_caps, convert, rate, rate_caps, sink, NULL);
  if (!gst_element_link_many (source, source_caps, convert, rate, rate_caps, sink, NULL)) {
    g_printerr ("could not link the test pipeline\n");
    return 2;
  }

  clock = gst_system_clock_obtain ();
  gst_pipeline_use_clock (GST_PIPELINE (pipeline), clock);
  gst_element_set_base_time (pipeline, 0);
  gst_element_set_start_time (pipeline, GST_CLOCK_TIME_NONE);

  gst_element_set_state (pipeline, GST_STATE_PLAYING);
  g_usleep (2 * G_USEC_PER_SEC);
  gst_element_set_state (pipeline, GST_STATE_NULL);

  g_print ("%u frames in 2 seconds\n", frames);
  gst_object_unref (pipeline);
  gst_object_unref (clock);
  return frames > 120 ? 1 : 0;
}
