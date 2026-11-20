@file:OptIn(
    androidx.media3.common.util.UnstableApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
)

package com.rmusic.nativeapp.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.*
import androidx.compose.animation.core.tween
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.*
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.*
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import com.rmusic.core.*
import com.rmusic.nativeapp.*
import kotlinx.coroutines.*
import org.json.JSONObject

// Route identity includes Core's immutable entity IDs, never an artist/album name match.
data class Page(
    val kind: String = "home",
    val id: String = "",
    val name: String = "",
    val path: String = "",
)

class Navigation {
    var pages by mutableStateOf(listOf(Page()))
    val current
        get() = pages.last()

    fun open(page: Page) {
        if (current != page) pages = (pages + page).takeLast(32)
    }

    companion object {
        val saver =
            androidx.compose.runtime.saveable.Saver<Navigation, String>(
                save = {
                    org.json
                        .JSONArray(
                            it.pages.map { p ->
                                json(
                                    "kind" to p.kind,
                                    "id" to p.id,
                                    "name" to p.name,
                                    "path" to p.path,
                                )
                            }
                        )
                        .toString()
                },
                restore = { v ->
                    Navigation().apply {
                        pages =
                            runCatching {
                                    org.json.JSONArray(v).objects().map {
                                        Page(
                                            it.text("kind"),
                                            it.text("id"),
                                            it.text("name"),
                                            it.text("path"),
                                        )
                                    }
                                }
                                .getOrDefault(listOf(Page()))
                    }
                },
            )
    }

    fun back() {
        if (pages.size > 1) pages = pages.dropLast(1)
    }

    fun root(kind: String) {
        pages = listOf(Page(kind))
    }
}

class Actions(val graph: AppGraph, val nav: Navigation) {
    var message by mutableStateOf("")
    var importFile by mutableStateOf<Pair<String, String>?>(null)
    var favorites by mutableStateOf<Set<String>>(emptySet())
    var selecting by mutableStateOf(false)
    var selection by mutableStateOf<Map<String, Track>>(emptyMap())

    fun toggle(t: Track) {
        selection = if (t.id in selection) selection - t.id else selection + (t.id to t)
    }

    fun run(block: suspend () -> Unit) {
        graph.scope.launch {
            try {
                block()
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                message = e.message ?: "操作失败"
            }
        }
    }

    fun play(rows: List<JSONObject>, index: Int = 0) = run {
        graph.runtime.play(rows.map(::Track), index)
    }

    fun entity(t: Track, kind: String, name: String = "") = run {
        val links =
            graph.api.value!!.request(
                "library/links",
                "POST",
                json(
                    "track_id" to t.id,
                    "resolve_credits" to true,
                    "artist" to t.artist,
                    "album" to t.album,
                    "source" to t.raw.text("source"),
                    "provider_data" to t.data,
                ),
            )
        val id =
            if (kind == "album") links.child("album").text("id")
            else links.rows("artists").firstOrNull { it.text("name") == name }?.text("id").orEmpty()
        require(id.isNotEmpty()) { "尚无明确的${kind}身份资料" }
        nav.open(Page(kind, id, if (kind == "album") t.album else name))
    }

    fun favorite(t: Track, liked: Boolean = t.id !in favorites) = run {
        val result =
            graph.api.value!!.request("favorites/${pathId(t.id)}", "PUT", json("liked" to liked))
        favorites =
            result
                .optJSONArray("track_ids")
                ?.let { v -> (0 until v.length()).map { v.optString(it) }.toSet() }
                .orEmpty()
        message = if (liked) "已加入我喜欢" else "已取消喜欢"
    }
}

@Composable
fun RMusicRoot(graph: AppGraph) {
    val api by graph.api.collectAsStateWithLifecycle()
    var theme by remember { mutableStateOf(graph.preferences.theme) }
    MineradioTheme(theme) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            AnimatedContent(api != null, label = "connection") { connected ->
                if (!connected) LoginScreen(graph)
                else
                    key(api?.session?.cacheNamespace) {
                        AppShell(graph) {
                            theme = it
                            graph.preferences.theme = it
                        }
                    }
            }
        }
    }
}

@Composable
private fun LoginScreen(graph: AppGraph) {
    var address by rememberSaveable {
        mutableStateOf(
            if (android.os.Build.MODEL.contains("SDK", true)) "http://10.0.2.2:18864" else ""
        )
    }
    var user by rememberSaveable { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var totp by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) { graph.vault.load() }
            ?.let {
                address = it.address
                user = it.username
            }
    }
    Box(
        Modifier.fillMaxSize()
            .background(
                Brush.radialGradient(
                    listOf(MaterialTheme.colorScheme.primary.copy(alpha = .14f), Color.Transparent)
                )
            )
            .systemBarsPadding(),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier.widthIn(max = 520.dp).verticalScroll(rememberScrollState()).padding(28.dp),
            verticalArrangement = Arrangement.spacedBy(22.dp),
        ) {
            Text("R · MUSIC", fontSize = 32.sp, fontWeight = FontWeight.Bold, letterSpacing = 4.sp)
            Text("音乐，随身而行。", fontSize = 24.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            GlassPanel {
                Text("连接 Core", style = MaterialTheme.typography.titleLarge)
                Spacer(Modifier.height(20.dp))
                OutlinedTextField(
                    address,
                    { address = it },
                    label = { Text("服务器地址") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                CoreDiscovery { address = it }
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(
                    user,
                    { user = it },
                    label = { Text("账号") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(
                    password,
                    { password = it },
                    label = { Text("密码") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(
                    totp,
                    { totp = it },
                    label = { Text("两步验证码（如已开启）") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                if (error.isNotBlank())
                    Text(
                        error,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(vertical = 12.dp),
                    )
                Spacer(Modifier.height(20.dp))
                Button(
                    onClick = {
                        busy = true
                        scope.launch {
                            try {
                                graph.login(address, user, password, totp)
                                password = ""
                            } catch (e: Exception) {
                                error = e.message ?: "连接失败"
                            } finally {
                                busy = false
                            }
                        }
                    },
                    enabled = !busy && user.isNotBlank() && password.isNotBlank(),
                    modifier = Modifier.fillMaxWidth().height(54.dp),
                ) {
                    if (busy) CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp)
                    else Text("进入音乐空间")
                }
            }
        }
    }
}

@Composable
private fun AppShell(graph: AppGraph, onTheme: (String) -> Unit) {
    val nav = rememberSaveable(saver = Navigation.saver) { Navigation() }
    val actions = remember { Actions(graph, nav) }
    LaunchedEffect(actions) {
        runCatching {
            val f = graph.api.value!!.request("favorites").optJSONArray("track_ids")
            actions.favorites =
                f?.let { v -> (0 until v.length()).map { v.optString(it) }.toSet() }.orEmpty()
        }
    }
    var full by rememberSaveable { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var search by rememberSaveable { mutableStateOf("") }
    val error by graph.connectionError.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(actions.message) {
        if (actions.message.isNotBlank()) {
            snackbar.showSnackbar(actions.message)
            actions.message = ""
        }
    }
    BackHandler(nav.pages.size > 1 && !full) { nav.back() }
    Scaffold(
        containerColor = Color.Transparent,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            Column(Modifier.statusBarsPadding()) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton({ if (nav.pages.size > 1) nav.back() else menu = true }) {
                        Icon(
                            if (nav.pages.size > 1) Icons.AutoMirrored.Rounded.ArrowBack
                            else Icons.Rounded.Menu,
                            "导航",
                        )
                    }
                    Text(
                        "R · MUSIC",
                        Modifier.weight(1f),
                        fontWeight = FontWeight.Bold,
                        letterSpacing = 2.sp,
                    )
                    IconButton({ nav.open(Page("devices")) }) {
                        Icon(Icons.Rounded.Speaker, "声音设备")
                    }
                    IconButton({ nav.open(Page("settings")) }) { Icon(Icons.Rounded.Tune, "本机设置") }
                }
                if (actions.selecting)
                    Row(
                        Modifier.fillMaxWidth().padding(horizontal = 20.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text("已选 ${actions.selection.size}", Modifier.weight(1f))
                        TextButton({
                            actions.selection.values.forEach { graph.downloads.enqueue(it) }
                            actions.message = "已加入下载队列"
                        }) {
                            Text("下载所选")
                        }
                        IconButton({
                            actions.selecting = false
                            actions.selection = emptyMap()
                        }) {
                            Icon(Icons.Rounded.Close, "结束多选")
                        }
                    }
                if (nav.current.kind !in listOf("settings", "devices")) {
                    SearchBarRow(search, { search = it }) {
                        if (search.isNotBlank()) nav.open(Page("search", search, search))
                    }
                }
            }
        },
        bottomBar = {
            Column(Modifier.navigationBarsPadding()) {
                MiniPlayer(graph, actions) { full = true }
                NavigationBar(containerColor = Color.Transparent, tonalElevation = 0.dp) {
                    listOf(
                            Triple("home", "首页", Icons.Rounded.Home),
                            Triple("library", "音乐空间", Icons.Rounded.Album),
                            Triple("playlists", "播放列表", Icons.AutoMirrored.Rounded.QueueMusic),
                        )
                        .forEach { (kind, label, icon) ->
                            NavigationBarItem(
                                nav.current.kind == kind,
                                { nav.root(kind) },
                                icon = { Icon(icon, label) },
                                label = { Text(label) },
                            )
                        }
                }
            }
        },
    ) { padding ->
        Box(
            Modifier.fillMaxSize()
                .padding(padding)
                .background(
                    Brush.verticalGradient(
                        listOf(
                            MaterialTheme.colorScheme.primary.copy(alpha = .025f),
                            Color.Transparent,
                        )
                    )
                )
        ) {
            AnimatedContent(
                nav.current,
                transitionSpec = { fadeIn(tween(180)) togetherWith fadeOut(tween(100)) },
                label = "page",
            ) { p ->
                when (p.kind) {
                    "home" -> HomePage(graph, actions)
                    "discover" -> DiscoveryPage(graph, actions)
                    "library",
                    "favorites" -> LibraryPage(graph, actions, p.kind)
                    "playlists" -> PlaylistsPage(graph, actions)
                    "import" -> PlaylistImportPage(graph, actions)
                    "search" -> SearchPage(graph, actions, p.id)
                    "artist",
                    "album" -> EntityPage(graph, actions, p)
                    "tracks" -> TracksPage(graph, actions, p)
                    "devices" -> DevicesPage(graph, actions)
                    "settings" -> SettingsPage(graph, actions, onTheme)
                    "downloads" -> DownloadsPage(graph, actions)
                    "stats" -> StatsPage(graph, actions)
                    "core" -> CorePage(graph, actions)
                }
            }
            if (error.isNotBlank())
                Surface(
                    Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(12.dp),
                    shape = RoundedCornerShape(16.dp),
                    color = MaterialTheme.colorScheme.errorContainer,
                ) {
                    Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(error, Modifier.weight(1f), maxLines = 2)
                        IconButton({ graph.report("") }) { Icon(Icons.Rounded.Close, "关闭") }
                    }
                }
        }
    }
    if (menu)
        ModalBottomSheet(
            onDismissRequest = { menu = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                Modifier.fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(bottom = 24.dp)
            ) {
                listOf(
                        "home" to "首页",
                        "discover" to "发现音乐",
                        "library" to "音乐空间",
                        "favorites" to "我喜欢",
                        "playlists" to "播放列表",
                        "stats" to "聆听统计",
                        "downloads" to "下载与缓存",
                        "devices" to "声音设备",
                        "settings" to "本机设置",
                        "core" to "Core 设置",
                    )
                    .forEach { (k, n) ->
                        TextButton(
                            {
                                nav.root(k)
                                menu = false
                            },
                            Modifier.fillMaxWidth().height(52.dp),
                        ) {
                            Text(n)
                        }
                    }
            }
        }
    if (full) PlayerScreen(graph, actions) { full = false }
}

@Composable
private fun SearchBarRow(value: String, onChange: (String) -> Unit, onSearch: () -> Unit) {
    val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    val submit = {
        keyboard?.hide()
        onSearch()
    }
    OutlinedTextField(
        value,
        onChange,
        Modifier.fillMaxWidth().padding(horizontal = 22.dp, vertical = 6.dp),
        placeholder = { Text("搜索歌曲、艺术家、专辑") },
        singleLine = true,
        shape = RoundedCornerShape(24.dp),
        leadingIcon = { IconButton(submit) { Icon(Icons.Rounded.Search, "搜索") } },
        trailingIcon = {
            if (value.isNotEmpty()) IconButton({ onChange("") }) { Icon(Icons.Rounded.Close, "清除") }
        },
        keyboardOptions =
            androidx.compose.foundation.text.KeyboardOptions(
                imeAction = androidx.compose.ui.text.input.ImeAction.Search
            ),
        keyboardActions = androidx.compose.foundation.text.KeyboardActions(onSearch = { submit() }),
    )
}

@Composable
fun Artwork(graph: AppGraph, path: String, modifier: Modifier = Modifier, round: Boolean = false) {
    val shape = if (round) CircleShape else RoundedCornerShape(14.dp)
    Box(
        modifier.clip(shape).background(MaterialTheme.colorScheme.surfaceVariant),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            Icons.Rounded.Album,
            null,
            Modifier.fillMaxSize(.38f),
            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .35f),
        )
        val url = graph.api.value?.image(path)
        if (url != null)
            AsyncImage(
                ImageRequest.Builder(graph.application)
                    .data(url)
                    .networkCachePolicy(
                        if (graph.runtime.selectedZone.value == "offline")
                            coil3.request.CachePolicy.DISABLED
                        else coil3.request.CachePolicy.ENABLED
                    )
                    .memoryCacheKey(
                        com.rmusic.nativeapp.artworkMemoryKey(
                            graph.api.value!!.session.cacheNamespace,
                            url,
                        )
                    )
                    .diskCacheKey(
                        com.rmusic.nativeapp.artworkCacheKey(
                            graph.api.value!!.session.cacheNamespace,
                            url,
                        )
                    )
                    .crossfade(140)
                    .build(),
                null,
                graph.images,
                Modifier.fillMaxSize(),
                contentScale = androidx.compose.ui.layout.ContentScale.Crop,
            )
    }
}

@Composable
fun PageTitle(title: String, subtitle: String = "", action: (@Composable () -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
            )
            if (subtitle.isNotBlank())
                Text(
                    subtitle,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    style = MaterialTheme.typography.bodySmall,
                )
        }
        action?.invoke()
    }
}

@Composable
fun Notice(text: String) {
    Text(text, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
fun Choice(
    label: String,
    current: String,
    options: List<Pair<String, String>>,
    change: (String) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        OutlinedButton({ open = true }) {
            Text(
                "$label · ${options.firstOrNull {it.first==current}?.second ?: current}",
                maxLines = 1,
            )
            Icon(Icons.Rounded.ExpandMore, null)
        }
        DropdownMenu(open, { open = false }) {
            options.forEach { (key, title) ->
                DropdownMenuItem(
                    text = { Text(title) },
                    onClick = {
                        change(key)
                        open = false
                    },
                )
            }
        }
    }
}
