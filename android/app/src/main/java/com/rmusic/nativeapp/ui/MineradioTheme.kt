package com.rmusic.nativeapp.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

val GlassShape = RoundedCornerShape(18.dp)

@Composable
fun MineradioTheme(variant:String="emerald",content:@Composable ()->Unit) {
    val accent=when(variant) {"violet"->Color(0xFFB9A3FF);"amber"->Color(0xFFEABD8A);"ocean"->Color(0xFF95C7F6);else->Color(0xFF8BE0BB)}
    val scheme=darkColorScheme(primary=accent,onPrimary=Color(0xFF101A17),secondary=accent,
        secondaryContainer=accent.copy(alpha=.13f),onSecondaryContainer=accent,
        background=Color(0xFF090C0F),surface=Color(0xFF15191D),surfaceVariant=Color(0xFF1C2227),
        onSurface=Color(0xFFF2F3F1),onSurfaceVariant=Color(0xFF8F999B),
        outline=Color(0xFF30383B),outlineVariant=Color(0xFF232A2E),surfaceTint=Color.Transparent,
        surfaceDim=Color(0xFF090C0F),surfaceBright=Color(0xFF282E32),surfaceContainerLowest=Color(0xFF090C0F),
        surfaceContainerLow=Color(0xFF11161A),surfaceContainer=Color(0xFF151A1F),surfaceContainerHigh=Color(0xFF1B2227),surfaceContainerHighest=Color(0xFF242B31))
    fun type(size:Int,line:Int,weight:FontWeight=FontWeight.Normal)=TextStyle(fontSize=size.sp,lineHeight=line.sp,fontWeight=weight,letterSpacing=(-.15).sp)
    MaterialTheme(colorScheme=scheme,shapes=Shapes(extraSmall=RoundedCornerShape(8.dp),small=RoundedCornerShape(12.dp),medium=RoundedCornerShape(16.dp),large=RoundedCornerShape(22.dp),extraLarge=RoundedCornerShape(28.dp)),
        typography=Typography(displayLarge=type(40,46,FontWeight.Bold),headlineLarge=type(28,34,FontWeight.Bold),headlineMedium=type(22,28,FontWeight.SemiBold),headlineSmall=type(20,26,FontWeight.SemiBold),titleLarge=type(19,25,FontWeight.SemiBold),titleMedium=type(15,21,FontWeight.SemiBold),titleSmall=type(13,19,FontWeight.Medium),bodyLarge=type(15,22),bodyMedium=type(13,20),bodySmall=type(11,17),labelLarge=type(13,18,FontWeight.SemiBold),labelMedium=type(11,16,FontWeight.Medium),labelSmall=type(10,14,FontWeight.Medium)),content=content)
}

@Composable
fun GlassPanel(modifier:Modifier=Modifier,content:@Composable ColumnScope.()->Unit) {
    Column(modifier.clip(GlassShape).background(MaterialTheme.colorScheme.surface.copy(alpha=.88f))
        .border(1.dp,Color.White.copy(alpha=.045f),GlassShape).padding(16.dp),content=content)
}
fun duration(seconds:Double):String {val n=seconds.toLong();return "%d:%02d".format(n/60,n%60)}
