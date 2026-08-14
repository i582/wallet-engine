package com.example.tonwallet.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

object AppSpacing {
    val xs = 4.dp
    val sm = 8.dp
    val md = 16.dp
    val lg = 24.dp
    val xl = 32.dp
}

val LocalAppSpacing = staticCompositionLocalOf { AppSpacing }

private val DarkColorScheme = darkColorScheme(
    primary = GramBlueDark,
    onPrimary = Midnight,
    primaryContainer = Color(0xFF073B66),
    onPrimaryContainer = Color(0xFFCDE8FF),
    secondary = Color(0xFF91C8FF),
    background = Midnight,
    surface = Color(0xFF101A26),
    surfaceVariant = Color(0xFF172536),
    onBackground = Color(0xFFF0F5FA),
    onSurface = Color(0xFFF0F5FA),
    outlineVariant = Color(0xFF2B3B4E),
    error = Color(0xFFFFB4AB),
)

private val LightColorScheme = lightColorScheme(
    primary = GramBlue,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFD9EDFF),
    onPrimaryContainer = Color(0xFF002E51),
    secondary = Color(0xFF27628F),
    background = Cloud,
    surface = Color.White,
    surfaceVariant = Color(0xFFEAF0F6),
    onBackground = Ink,
    onSurface = Ink,
    outlineVariant = Color(0xFFD8E1EA),
    error = Color(0xFFBA1A1A),
)

@Composable
fun TONWalletTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme) DarkColorScheme else LightColorScheme,
        typography = Typography,
        shapes = AppShapes,
        content = content,
    )
}
