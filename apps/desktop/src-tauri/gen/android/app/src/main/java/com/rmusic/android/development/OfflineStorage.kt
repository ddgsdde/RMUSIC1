package com.rmusic.android.development

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.system.Os
import android.system.OsConstants
import org.json.JSONObject

/** Persisted SAF grants keep downloads usable across Activity and process restarts. */
class OfflineStorage(private val context: Context) {
    private val resolver get() = context.contentResolver
    private val preferences get() = context.getSharedPreferences("rmusic-offline-storage",Context.MODE_PRIVATE)
    private fun root(): Uri? = preferences.getString("tree",null)?.takeIf { it.isNotBlank() }?.let(Uri::parse)
    private fun document(tree: Uri) = DocumentsContract.buildDocumentUriUsingTree(tree,DocumentsContract.getTreeDocumentId(tree))
    private fun title(tree: Uri): String {
        resolver.query(document(tree),arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),null,null,null)?.use {
            check(it.moveToFirst()) { "所选目录不存在，请重新选择目录或连接存储设备" }
            return it.getString(0)
        }
        error("无法读取所选离线目录")
    }
    fun status(): JSONObject {
        val tree=root()
        val result=JSONObject().put("tree",tree?.toString()?:JSONObject.NULL)
            .put("name",preferences.getString("name","应用内存储"))
        if(tree==null)return result.put("available",true)
        return try {
            check(resolver.persistedUriPermissions.any { it.uri==tree&&it.isReadPermission&&it.isWritePermission }) { "离线目录授权已失效，请重新授权原目录" }
            result.put("name",title(tree)).put("available",true)
        }catch(error:Exception){result.put("available",false).put("error",error.message?:"离线目录不可访问，请连接存储设备或重新授权")}
    }
    fun chooseIntent(): Intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
        root()?.let { putExtra(DocumentsContract.EXTRA_INITIAL_URI,document(it)) }
    }
    fun accept(tree: Uri,flags: Int): JSONObject {
        resolver.takePersistableUriPermission(tree,flags and (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION))
        val name=title(tree)
        val probe=DocumentsContract.createDocument(resolver,document(tree),"application/octet-stream",".rmusic-check-${java.util.UUID.randomUUID()}")?:error("所选目录不可写")
        try {
            resolver.openFileDescriptor(probe,"rw")!!.use {
                val data=byteArrayOf(82,77,0)
                Os.write(it.fileDescriptor,data,0,data.size);Os.lseek(it.fileDescriptor,0,OsConstants.SEEK_SET)
                val actual=ByteArray(data.size);check(Os.read(it.fileDescriptor,actual,0,actual.size)==data.size&&actual.contentEquals(data)) { "所选目录不能完整读写离线音乐" }
            }
        } finally {DocumentsContract.deleteDocument(resolver,probe)}
        preferences.edit().putString("tree",tree.toString()).putString("name",name).commit()
        return status().put("cancelled",false)
    }
    fun command(command: String,args: JSONObject): String = try {
        when(command) {
            "storage_status" -> status()
            "storage_revoke" -> {check(BuildConfig.DEBUG);val tree=root()?:error("未选择目录");resolver.releasePersistableUriPermission(tree,Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION);status()}
            "storage_reset" -> {preferences.edit().remove("tree").remove("name").commit();status()}
            "storage_create" -> {
                val tree=Uri.parse(args.getString("tree"))
                val name=args.getString("name").replace(Regex("[\\/:*?\"<>|\u0000]"),"_")
                val uri=DocumentsContract.createDocument(resolver,document(tree),"application/octet-stream",name)?:error("所选目录无法创建离线文件")
                JSONObject().put("uri",uri.toString())
            }
            "storage_open" -> {
                val uri=Uri.parse(args.getString("uri"))
                val descriptor=resolver.openFileDescriptor(uri,if(args.optBoolean("write"))"rw" else "r")?:error("离线文件不可访问")
                descriptor.use {Os.lseek(it.fileDescriptor,0,OsConstants.SEEK_SET);JSONObject().put("fd",it.detachFd())}
            }
            "storage_remove" -> {
                val uri=Uri.parse(args.getString("uri"))
                try {check(DocumentsContract.deleteDocument(resolver,uri)) { "无法移除离线文件" }}
                catch(error:java.io.FileNotFoundException){title(uri)}
                JSONObject().put("removed",true)
            }
            else -> error("未知离线存储操作")
        }.toString()
    }catch(error:Exception){JSONObject().put("error","离线目录不可访问，请连接存储设备或重新授权：${error.message}").toString()}
}
