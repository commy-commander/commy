#include <gst/gst.h>

#define COMMY_TYPE_CAMERA_FIX (commy_camera_fix_get_type ())
G_DECLARE_FINAL_TYPE (CommyCameraFix, commy_camera_fix, COMMY, CAMERA_FIX, GstTracer)

struct _CommyCameraFix
{
  GstTracer parent;
};

G_DEFINE_TYPE (CommyCameraFix, commy_camera_fix, GST_TYPE_TRACER)

static gboolean
is_dma_drm_in_system_memory (GstCaps * caps, guint index)
{
  GstStructure *structure = gst_caps_get_structure (caps, index);
  GstCapsFeatures *features = gst_caps_get_features (caps, index);

  return g_strcmp0 (gst_structure_get_string (structure, "format"), "DMA_DRM") == 0
      && !gst_caps_features_contains (features, "memory:DMABuf");
}

static gboolean
asks_dma_drm_in_system_memory (GstCaps * caps)
{
  for (guint i = 0; i < gst_caps_get_size (caps); i++) {
    if (is_dma_drm_in_system_memory (caps, i))
      return TRUE;
  }
  return FALSE;
}

static GstCaps *
without_dma_drm_format (GstCaps * caps)
{
  GstCaps *fixed = gst_caps_copy (caps);

  for (guint i = 0; i < gst_caps_get_size (fixed); i++) {
    if (is_dma_drm_in_system_memory (fixed, i))
      gst_structure_remove_fields (gst_caps_get_structure (fixed, i), "format", "drm-format", NULL);
  }
  return fixed;
}

static void
on_caps_changed (GObject * filter, GParamSpec * pspec, gpointer user_data)
{
  GstCaps *caps = NULL;

  g_object_get (filter, "caps", &caps, NULL);
  if (caps == NULL)
    return;
  if (asks_dma_drm_in_system_memory (caps)) {
    GstCaps *fixed = without_dma_drm_format (caps);
    GST_INFO_OBJECT (filter, "replacing %" GST_PTR_FORMAT " with %" GST_PTR_FORMAT, caps, fixed);
    g_object_set (filter, "caps", fixed, NULL);
    gst_caps_unref (fixed);
  }
  gst_caps_unref (caps);
}

static void
on_element_new (GstTracer * tracer, GstClockTime ts, GstElement * element)
{
  GstElementFactory *factory = gst_element_get_factory (element);

  if (factory != NULL && g_strcmp0 (GST_OBJECT_NAME (factory), "capsfilter") == 0)
    g_signal_connect (element, "notify::caps", G_CALLBACK (on_caps_changed), NULL);
}

static void
commy_camera_fix_class_init (CommyCameraFixClass * klass)
{
}

static void
commy_camera_fix_init (CommyCameraFix * self)
{
  gst_tracing_register_hook (GST_TRACER (self), "element-new", G_CALLBACK (on_element_new));
}

static gboolean
plugin_init (GstPlugin * plugin)
{
  return gst_tracer_register (plugin, "commycamerafix", COMMY_TYPE_CAMERA_FIX);
}

GST_PLUGIN_DEFINE (GST_VERSION_MAJOR, GST_VERSION_MINOR, commycamerafix,
    "Keeps camera capture in a format the camera can deliver",
    plugin_init, "1.0", "unknown", "Commy", "https://commy.chat")
