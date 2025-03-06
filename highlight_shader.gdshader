shader_type canvas_item;

// Fill color for the background
uniform vec4 fill_color : source_color = vec4(0.0, 0.0, 0.0, 0.0);
uniform vec4 highlight_color : source_color = vec4(0.0, 0.0, 0.0, 0.0);

// Maximum number of rectangles we can handle
const int MAX_RECTANGLES = 2500;

// Array of rectangles (x, y, width, height)
uniform vec4 rectangles[MAX_RECTANGLES];

// Number of active rectangles
uniform int rectangle_count = 0;

// Maximum distance for the opacity transition
uniform float max_distance = 0.0075;

// Sharpness factor for the gradient (higher = sharper)
uniform float sharpness = 3.0;

bool point_in_rectangle(vec2 point, vec4 rect) {
    // rect format: (x, y, width, height)
    return point.x >= rect.x &&
           point.x <= rect.x + rect.z &&
           point.y >= rect.y &&
           point.y <= rect.y + rect.w;
}

float distance_to_rectangle(vec2 point, vec4 rect) {
    // If the point is inside the rectangle, return 0
    if (point_in_rectangle(point, rect)) {
        return 0.0;
    }

    // Calculate the closest point on the rectangle to the given point
    vec2 closest_point;

    // Clamp the point to the rectangle's bounds
    closest_point.x = max(rect.x, min(point.x, rect.x + rect.z));
    closest_point.y = max(rect.y, min(point.y, rect.y + rect.w));

    // Return the distance to the closest point
    return distance(point, closest_point);
}

void fragment() {
    // Default to the fill color
    vec4 color = fill_color;

    // Track the minimum distance to any rectangle
    float min_distance = max_distance;

    for (int i = 0; i < rectangle_count; i++) {
        // Calculate distance to this rectangle
        float dist = distance_to_rectangle(UV, rectangles[i]);
        min_distance = min(min_distance, dist);
    }

    if (min_distance < max_distance) {
        // Calculate opacity based on distance
        // The closer to the rectangle, the more transparent
        float opacity_factor = min_distance / max_distance;

        // Apply sharpness curve to the opacity transition
        opacity_factor = pow(opacity_factor, sharpness);

        // Create a version of highlight_color with opacity based on distance
        vec4 adjusted_highlight = highlight_color;
        adjusted_highlight.a = highlight_color.a * opacity_factor;

        // Mix between adjusted highlight color and fill color
        color = mix(adjusted_highlight, fill_color, opacity_factor);
    }

    COLOR = color;
}