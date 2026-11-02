use serde_json::Value;

pub const LOCAL_VOICE_KEY: &str = "commy.web.voice";
const LOCAL_VOICE_FIELDS: [&str; 9] = ["noise_cancellation", "echo_cancellation", "auto_level", "output_volume", "input_gain", "input_device", "output_device", "user_volumes", "muted_users"];

pub fn overlay_local_voice(settings: &mut Value, local: Option<Value>) {
    let (Some(settings), Some(Value::Object(local))) = (settings.as_object_mut(), local) else { return };
    for field in LOCAL_VOICE_FIELDS {
        let Some(value) = local.get(field) else { continue };
        let fits = match settings.get(field) {
            Some(Value::Bool(_)) => value.is_boolean(),
            Some(Value::Number(_)) => value.is_number(),
            Some(Value::Object(_)) => value.is_object(),
            Some(Value::Array(_)) => value.is_array(),
            _ => value.is_string() || value.is_null(),
        };
        if fits {
            settings.insert(field.to_string(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({ "noise_cancellation": true, "output_volume": 1, "input_device": null, "user_volumes": {}, "muted_users": [], "muted": false })
    }

    #[test]
    fn this_browser_s_voice_choices_show_in_the_settings() {
        let mut settings = base();
        overlay_local_voice(&mut settings, Some(json!({ "noise_cancellation": false, "output_volume": 0.5, "input_device": "Headset", "user_volumes": { "a": 1.5 }, "muted_users": ["b"] })));
        assert_eq!(settings["noise_cancellation"], false);
        assert_eq!(settings["output_volume"], 0.5);
        assert_eq!(settings["input_device"], "Headset");
        assert_eq!(settings["user_volumes"]["a"], 1.5);
        assert_eq!(settings["muted_users"][0], "b");
    }

    #[test]
    fn stored_choices_cannot_change_call_state_or_the_shape_of_a_setting() {
        let mut settings = base();
        overlay_local_voice(&mut settings, Some(json!({ "muted": true, "noise_cancellation": "no", "output_volume": "loud", "user_volumes": [1] })));
        assert_eq!(settings, base());
        overlay_local_voice(&mut settings, Some(json!("not an object")));
        overlay_local_voice(&mut settings, None);
        assert_eq!(settings, base());
    }
}
