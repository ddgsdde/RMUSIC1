@file:OptIn(androidx.media3.common.util.UnstableApi::class)
package com.rmusic.nativeapp.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*

@Composable
fun BrandMark(modifier:Modifier=Modifier) {
    val color=MaterialTheme.colorScheme.primary
    Canvas(modifier.size(23.dp)) {
        val heights=listOf(.42f,.78f,1f,.64f)
        heights.forEachIndexed {i,h->val x=size.width*(.13f+i*.23f);val y=(size.height-size.height*h)/2
            drawLine(color,Offset(x+size.width*.07f,y),Offset(x-size.width*.07f,size.height-y),strokeWidth=size.width*.115f,cap=StrokeCap.Round)
        }
    }
}

@Composable
fun StudioTabs(current:String,options:List<Pair<String,String>>,change:(String)->Unit,modifier:Modifier=Modifier) {
    Row(modifier.horizontalScroll(rememberScrollState()).heightIn(min=46.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(22.dp)) {
        options.forEach {(id,label)->
            val selected=current==id
            val color by animateColorAsState(if(selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,tween(150),label="tab")
            Column(Modifier.clip(RoundedCornerShape(4.dp)).clickable {change(id)}.height(46.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.Center) {
                Text(label,color=color,fontWeight=if(selected)FontWeight.Bold else FontWeight.Medium,fontSize=16.sp)
                Spacer(Modifier.height(6.dp))
                Box(Modifier.width(16.dp).height(2.dp).background(if(selected)MaterialTheme.colorScheme.primary else Color.Transparent,CircleShape))
            }
        }
    }
}

@Composable
fun PillAction(title:String,icon:ImageVector?=null,active:Boolean=false,enabled:Boolean=true,onClick:()->Unit) {
    Row(Modifier.heightIn(min=40.dp).clip(RoundedCornerShape(13.dp))
        .background(if(active)MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant.copy(alpha=.7f))
        .clickable(enabled=enabled,onClick=onClick).padding(horizontal=13.dp,vertical=9.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(6.dp)) {
        val color=if(!enabled)MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha=.4f) else if(active)MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
        if(icon!=null) Icon(icon,null,Modifier.size(17.dp),tint=color)
        Text(title,color=color,style=MaterialTheme.typography.labelLarge,maxLines=1)
    }
}

@Composable
fun StudioField(value:String,change:(String)->Unit,label:String,modifier:Modifier=Modifier,visualTransformation:VisualTransformation=VisualTransformation.None,options:KeyboardOptions=KeyboardOptions.Default,submit:(()->Unit)?=null) {
    Column(modifier,verticalArrangement=Arrangement.spacedBy(6.dp)) {
        Text(label,color=MaterialTheme.colorScheme.onSurfaceVariant,style=MaterialTheme.typography.labelMedium)
        BasicTextField(value,change,Modifier.fillMaxWidth().heightIn(min=48.dp).clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha=.65f)).padding(horizontal=14.dp,vertical=13.dp),
            textStyle=MaterialTheme.typography.bodyLarge.copy(color=MaterialTheme.colorScheme.onSurface),singleLine=true,
            visualTransformation=visualTransformation,keyboardOptions=options,
            keyboardActions=KeyboardActions(onDone={submit?.invoke()},onSearch={submit?.invoke()}))
    }
}

@Composable
fun SmallCaption(value:String,modifier:Modifier=Modifier) {
    Text(value,modifier,color=MaterialTheme.colorScheme.onSurfaceVariant,style=MaterialTheme.typography.labelSmall,maxLines=1,overflow=TextOverflow.Ellipsis)
}

@Composable
fun SearchBarRow(value:String,onChange:(String)->Unit,onSearch:()->Unit) {
    val keyboard=androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    val submit={keyboard?.hide();onSearch()}
    Row(Modifier.fillMaxWidth().padding(horizontal=20.dp,vertical=5.dp).height(44.dp).clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha=.6f)),verticalAlignment=Alignment.CenterVertically) {
        IconButton(submit,Modifier.size(44.dp)){Icon(Icons.Rounded.Search,"搜索",Modifier.size(19.dp),tint=MaterialTheme.colorScheme.onSurfaceVariant)}
        BasicTextField(value,onChange,Modifier.weight(1f),textStyle=MaterialTheme.typography.bodyMedium.copy(color=MaterialTheme.colorScheme.onSurface),singleLine=true,
            keyboardOptions=KeyboardOptions(imeAction=ImeAction.Search),keyboardActions=KeyboardActions(onSearch={submit()}),
            decorationBox={inner->Box {if(value.isEmpty())Text("搜索歌曲、艺术家、专辑",color=MaterialTheme.colorScheme.onSurfaceVariant,style=MaterialTheme.typography.bodyMedium);inner()}})
        if(value.isNotEmpty())IconButton({onChange("")},Modifier.size(44.dp)){Icon(Icons.Rounded.Close,"清除",Modifier.size(17.dp))}
    }
}
